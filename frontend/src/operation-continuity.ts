import type { BrokerJob } from './control-api';

const STORAGE_KEY = 'helix.operations.v1';
const MAX_RECORDS = 24;
const RETENTION_MS = 7 * 24 * 60 * 60 * 1_000;
const JOB_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu;

export const OPERATION_CONTINUITY_EVENT = 'helix:operation-continuity';
let inFlightMutationCount = 0;
let refreshGuardInstalled = false;

function refreshGuard(event: BeforeUnloadEvent): void {
  if (inFlightMutationCount === 0) return;
  event.preventDefault();
  event.returnValue = '';
}

export function beginMutationRequest(): void {
  inFlightMutationCount += 1;
  if (!refreshGuardInstalled && typeof window !== 'undefined') {
    window.addEventListener('beforeunload', refreshGuard);
    refreshGuardInstalled = true;
  }
}

export function endMutationRequest(): void {
  inFlightMutationCount = Math.max(0, inFlightMutationCount - 1);
}

export function mutationRefreshIsUnsafe(): boolean {
  return inFlightMutationCount > 0;
}

export interface ResumableOperation {
  id: string;
  label: string;
  status: BrokerJob['status'];
  stage: string;
  progressPercent: number;
  createdAtUnixMs: number;
  updatedAtUnixMs: number;
  error: string | null;
}

interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

function browserStorage(): StorageLike | null {
  if (typeof window === 'undefined') return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function validRecord(value: unknown, now: number): value is ResumableOperation {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return typeof record.id === 'string' && JOB_ID.test(record.id)
    && typeof record.label === 'string' && record.label.length > 0 && record.label.length <= 80
    && ['queued', 'running', 'complete', 'failed'].includes(String(record.status))
    && typeof record.stage === 'string' && record.stage.length <= 256
    && typeof record.progressPercent === 'number' && Number.isInteger(record.progressPercent)
    && record.progressPercent >= 0 && record.progressPercent <= 100
    && typeof record.createdAtUnixMs === 'number' && Number.isSafeInteger(record.createdAtUnixMs)
    && typeof record.updatedAtUnixMs === 'number' && Number.isSafeInteger(record.updatedAtUnixMs)
    && record.createdAtUnixMs > 0 && record.updatedAtUnixMs >= record.createdAtUnixMs
    && now - record.updatedAtUnixMs <= RETENTION_MS
    && (record.error === null || (typeof record.error === 'string' && record.error.length <= 2_048));
}

export function readResumableOperations(
  storage: StorageLike | null = browserStorage(),
  now = Date.now(),
): ResumableOperation[] {
  if (storage === null) return [];
  try {
    const parsed: unknown = JSON.parse(storage.getItem(STORAGE_KEY) ?? '[]');
    if (!Array.isArray(parsed)) throw new Error('invalid operation journal');
    const records = parsed.filter((item) => validRecord(item, now)).slice(0, MAX_RECORDS);
    if (records.length === 0) storage.removeItem(STORAGE_KEY);
    else storage.setItem(STORAGE_KEY, JSON.stringify(records));
    return records;
  } catch {
    try { storage.removeItem(STORAGE_KEY); } catch { /* Storage is unavailable. */ }
    return [];
  }
}

function writeRecords(records: ResumableOperation[], storage: StorageLike | null): void {
  if (storage === null) return;
  try {
    if (records.length === 0) storage.removeItem(STORAGE_KEY);
    else storage.setItem(STORAGE_KEY, JSON.stringify(records.slice(0, MAX_RECORDS)));
  } catch {
    // Background work continues server-side even when browser storage is blocked.
  }
}

function notify(): void {
  if (typeof window !== 'undefined') window.dispatchEvent(new Event(OPERATION_CONTINUITY_EVENT));
}

function operationLabel(path: string): string {
  if (path.includes('/marketplace/')) return 'Marketplace install';
  if (path.includes('/modpack') && path.includes('/update')) return 'Modpack update';
  if (path.includes('/modpack')) return 'Minecraft modpack creation';
  if (path.includes('/migrate')) return 'Server copy';
  if (path.includes('/backups') && path.includes('/restore')) return 'Backup restore';
  if (path.includes('/backups')) return 'Server backup';
  if (path.includes('/hooks/')) return 'Hook install';
  if (path.includes('/system/packages')) return 'System update';
  if (path.includes('/system/helix')) return 'Helix update';
  if (path.includes('/servers')) return 'Server operation';
  return 'Background operation';
}

export function rememberOperationDispatch(
  path: string,
  value: unknown,
  storage: StorageLike | null = browserStorage(),
  now = Date.now(),
): boolean {
  if (path.includes('/storage/analysis/')) return false;
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  const result = value as Record<string, unknown>;
  const candidate = typeof result.jobId === 'string' ? result.jobId : result.job_id;
  if (typeof candidate !== 'string' || !JOB_ID.test(candidate)) return false;
  const id = candidate.toLowerCase();
  const previous = readResumableOperations(storage, now).filter((item) => item.id !== id);
  writeRecords([{
    id,
    label: operationLabel(path),
    status: 'queued',
    stage: 'Queued safely on the server',
    progressPercent: 0,
    createdAtUnixMs: now,
    updatedAtUnixMs: now,
    error: null,
  }, ...previous], storage);
  notify();
  return true;
}

export function updateResumableOperation(
  job: BrokerJob,
  storage: StorageLike | null = browserStorage(),
): void {
  const records = readResumableOperations(storage);
  const existing = records.find((item) => item.id === job.id);
  if (existing === undefined) return;
  writeRecords(records.map((item) => item.id === job.id ? {
    ...item,
    status: job.status,
    stage: job.stage.slice(0, 256),
    progressPercent: job.progressPercent,
    updatedAtUnixMs: Math.max(item.createdAtUnixMs, job.updatedAtUnixMs),
    error: job.error?.slice(0, 2_048) ?? null,
  } : item), storage);
  notify();
}

export function forgetResumableOperation(
  id: string,
  storage: StorageLike | null = browserStorage(),
): void {
  writeRecords(readResumableOperations(storage).filter((item) => item.id !== id), storage);
  notify();
}
