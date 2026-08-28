import {
  ApiError,
  expectArray,
  expectNumber,
  expectRecord,
  expectString,
  requestJson,
  type JsonRecord,
} from './api';

const MAX_PATH_BYTES = 4_096;
const MAX_NAME_BYTES = 255;
const MAX_RESULTS_PER_LIST = 2_048;
const MAX_ENTRIES = 5_000_000;
const MAX_DEPTH = 128;
const MAX_DURATION_MS = 600_000;
const MAX_RESPONSE_BYTES = 8_388_608;
const U64_MAX = 18_446_744_073_709_551_615n;

export type StorageAnalysisJobState =
  | 'queued'
  | 'running'
  | 'complete'
  | 'cancelled'
  | 'failed';
export type StorageAnalysisStopReason = 'cancelled' | 'duration_limit' | 'entry_limit';
export type StorageAnalysisMode = 'quick' | 'thorough';

export interface StorageAnalysisStart {
  jobId: string;
  state: 'queued';
}

export interface StorageAnalysisProgress {
  percent: number | null;
  entryBudgetPercent: number;
  durationBudgetPercent: number;
  entriesScanned: number;
  filesScanned: number;
  directoriesScanned: number;
  bytesScanned: bigint;
  allocatedBytesScanned: bigint;
}

export interface StorageAnalysisFile {
  path: string;
  name: string;
  type: 'file';
  bytes: bigint;
  allocatedBytes: bigint;
}

export interface StorageAnalysisFolder {
  path: string;
  name: string;
  type: 'directory';
  immediateBytes: bigint;
  recursiveBytes: bigint;
  immediateAllocatedBytes: bigint;
  recursiveAllocatedBytes: bigint;
  immediateComplete: boolean;
  recursiveComplete: boolean;
}

export interface StorageAnalysisErrors {
  total: number;
  permissionDenied: number;
  filesystemRaces: number;
  metadataFailures: number;
  sizeOverflows: number;
}

export interface StorageAnalysisSkipped {
  restrictedPaths: number;
  symbolicLinks: number;
  otherFilesystems: number;
  specialFiles: number;
  depthLimitedDirectories: number;
  unrepresentableNames: number;
  hardLinkAliases: number;
}

export interface StorageAnalysisLimits {
  mode: StorageAnalysisMode;
  maxDurationMs: number;
  maxEntries: number;
  maxDepth: number;
  maxResultsPerList: number;
  maxResponseBytes: number;
  stayOnTargetFilesystem: true;
  followsSymbolicLinks: false;
}

export interface StorageAnalysisResult {
  rootPath: string;
  apparentBytesScanned: bigint;
  allocatedBytesScanned: bigint;
  truncated: boolean;
  stopReason: StorageAnalysisStopReason | null;
  errors: StorageAnalysisErrors;
  skipped: StorageAnalysisSkipped;
  fileResultsOmitted: number;
  recursiveFolderResultsOmitted: number;
  immediateFolderResultsOmitted: number;
  responseResultsOmitted: number;
  largestFiles: StorageAnalysisFile[];
  largestFoldersByRecursiveBytes: StorageAnalysisFolder[];
  largestFoldersByImmediateBytes: StorageAnalysisFolder[];
  limits: StorageAnalysisLimits;
}

export interface StorageAnalysisStatus {
  jobId: string;
  requestedPath: string;
  state: StorageAnalysisJobState;
  progress: StorageAnalysisProgress;
  createdUnixMs: number;
  startedUnixMs: number | null;
  finishedUnixMs: number | null;
  cancelRequested: boolean;
  result: StorageAnalysisResult | null;
  error: string | null;
}

function invalid(context: string, key: string): never {
  throw new ApiError(`${context} returned an invalid ${key} value.`);
}

function boolean(record: JsonRecord, key: string, context: string): boolean {
  const value = record[key];
  if (typeof value !== 'boolean') invalid(context, key);
  return value;
}

function nullableNumber(
  record: JsonRecord,
  key: string,
  context: string,
  maximum = Number.MAX_SAFE_INTEGER,
): number | null {
  if (record[key] === null) return null;
  return expectNumber(record, key, context, { integer: true, minimum: 0, maximum });
}

function count(record: JsonRecord, key: string, context: string, maximum = MAX_ENTRIES): number {
  return expectNumber(record, key, context, { integer: true, minimum: 0, maximum });
}

function u64(record: JsonRecord, key: string, context: string): bigint {
  const value = record[key];
  if (typeof value !== 'string' || !/^(?:0|[1-9]\d*)$/u.test(value) || value.length > 20) {
    invalid(context, key);
  }
  const parsed = BigInt(value);
  if (parsed > U64_MAX) invalid(context, key);
  return parsed;
}

function hasControlCharacters(value: string): boolean {
  return Array.from(value).some((character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint <= 0x1f || codePoint === 0x7f;
  });
}

function safePath(record: JsonRecord, key: string, context: string): string {
  const value = expectString(record, key, context);
  const segments = value.split('/');
  if (
    !value.startsWith('/') ||
    new TextEncoder().encode(value).length > MAX_PATH_BYTES ||
    hasControlCharacters(value) ||
    value.includes('//') ||
    (value !== '/' && value.endsWith('/')) ||
    segments.some((segment) => segment === '.' || segment === '..')
  ) {
    invalid(context, key);
  }
  return value;
}

function safeName(
  record: JsonRecord,
  key: string,
  context: string,
  path: string,
): string {
  const value = expectString(record, key, context);
  const expected = path === '/' ? '/' : path.slice(path.lastIndexOf('/') + 1);
  if (
    new TextEncoder().encode(value).length > MAX_NAME_BYTES ||
    hasControlCharacters(value) ||
    (value.includes('/') && value !== '/') ||
    value === '.' ||
    value === '..' ||
    value !== expected
  ) {
    invalid(context, key);
  }
  return value;
}

function uuid(record: JsonRecord, key: string, context: string): string {
  const value = expectString(record, key, context);
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu.test(value)) {
    invalid(context, key);
  }
  return value.toLowerCase();
}

function state(record: JsonRecord, context: string): StorageAnalysisJobState {
  const value = record.state;
  if (!['queued', 'running', 'complete', 'cancelled', 'failed'].includes(String(value))) {
    invalid(context, 'state');
  }
  return value as StorageAnalysisJobState;
}

function nullableError(record: JsonRecord, context: string): string | null {
  if (record.error === null) return null;
  const value = expectString(record, 'error', context);
  if (new TextEncoder().encode(value).length > 1_024 || hasControlCharacters(value)) {
    invalid(context, 'error');
  }
  return value;
}

function parseProgress(value: unknown): StorageAnalysisProgress {
  const context = 'Storage analysis progress';
  const record = expectRecord(value, context);
  return {
    percent: nullableNumber(record, 'percent', context, 100),
    entryBudgetPercent: count(record, 'entry_budget_percent', context, 100),
    durationBudgetPercent: count(record, 'duration_budget_percent', context, 100),
    entriesScanned: count(record, 'entries_scanned', context),
    filesScanned: count(record, 'files_scanned', context),
    directoriesScanned: count(record, 'directories_scanned', context, MAX_ENTRIES + 1),
    bytesScanned: u64(record, 'bytes_scanned', context),
    allocatedBytesScanned: u64(record, 'allocated_bytes_scanned', context),
  };
}

function parseFile(value: unknown, index: number): StorageAnalysisFile {
  const context = `Storage analysis file ${index + 1}`;
  const record = expectRecord(value, context);
  if (record.type !== 'file') invalid(context, 'type');
  const path = safePath(record, 'path', context);
  return {
    path,
    name: safeName(record, 'name', context, path),
    type: 'file',
    bytes: u64(record, 'bytes', context),
    allocatedBytes: u64(record, 'allocated_bytes', context),
  };
}

function parseFolder(value: unknown, index: number, list: string): StorageAnalysisFolder {
  const context = `Storage analysis ${list} folder ${index + 1}`;
  const record = expectRecord(value, context);
  if (record.type !== 'directory') invalid(context, 'type');
  const path = safePath(record, 'path', context);
  return {
    path,
    name: safeName(record, 'name', context, path),
    type: 'directory',
    immediateBytes: u64(record, 'immediate_bytes', context),
    recursiveBytes: u64(record, 'recursive_bytes', context),
    immediateAllocatedBytes: u64(record, 'immediate_allocated_bytes', context),
    recursiveAllocatedBytes: u64(record, 'recursive_allocated_bytes', context),
    immediateComplete: boolean(record, 'immediate_complete', context),
    recursiveComplete: boolean(record, 'recursive_complete', context),
  };
}

function parseErrors(value: unknown): StorageAnalysisErrors {
  const context = 'Storage analysis errors';
  const record = expectRecord(value, context);
  const errors = {
    total: count(record, 'total', context),
    permissionDenied: count(record, 'permission_denied', context),
    filesystemRaces: count(record, 'filesystem_races', context),
    metadataFailures: count(record, 'metadata_failures', context),
    sizeOverflows: count(record, 'size_overflows', context),
  };
  if (
    errors.total !==
    errors.permissionDenied + errors.filesystemRaces + errors.metadataFailures + errors.sizeOverflows
  ) {
    invalid(context, 'total');
  }
  return errors;
}

function parseSkipped(value: unknown): StorageAnalysisSkipped {
  const context = 'Storage analysis skipped entries';
  const record = expectRecord(value, context);
  return {
    restrictedPaths: count(record, 'restricted_paths', context),
    symbolicLinks: count(record, 'symbolic_links', context),
    otherFilesystems: count(record, 'other_filesystems', context),
    specialFiles: count(record, 'special_files', context),
    depthLimitedDirectories: count(record, 'depth_limited_directories', context),
    unrepresentableNames: count(record, 'unrepresentable_names', context),
    hardLinkAliases: count(record, 'hard_link_aliases', context),
  };
}

function parseLimits(value: unknown): StorageAnalysisLimits {
  const context = 'Storage analysis limits';
  const record = expectRecord(value, context);
  if (record.mode !== 'quick' && record.mode !== 'thorough') invalid(context, 'mode');
  const limits = {
    mode: record.mode as StorageAnalysisMode,
    maxDurationMs: count(record, 'max_duration_ms', context, MAX_DURATION_MS),
    maxEntries: count(record, 'max_entries', context, MAX_ENTRIES),
    maxDepth: count(record, 'max_depth', context, MAX_DEPTH),
    maxResultsPerList: count(record, 'max_results_per_list', context, MAX_RESULTS_PER_LIST),
    maxResponseBytes: count(record, 'max_response_bytes', context, MAX_RESPONSE_BYTES),
    stayOnTargetFilesystem: boolean(record, 'stay_on_target_filesystem', context),
    followsSymbolicLinks: boolean(record, 'follows_symbolic_links', context),
  };
  if (
    limits.maxDurationMs === 0 ||
    limits.maxEntries === 0 ||
    limits.maxResultsPerList === 0 ||
    limits.maxResponseBytes === 0 ||
    limits.stayOnTargetFilesystem !== true ||
    limits.followsSymbolicLinks !== false
  ) {
    throw new ApiError(`${context} returned unsafe bounds.`);
  }
  return limits as StorageAnalysisLimits;
}

function uniquePaths<T extends { path: string }>(items: T[], context: string): T[] {
  if (new Set(items.map((item) => item.path)).size !== items.length) {
    throw new ApiError(`${context} returned duplicate paths.`);
  }
  return items;
}

function parseResult(value: unknown): StorageAnalysisResult {
  const context = 'Storage analysis result';
  const record = expectRecord(value, context);
  const limits = parseLimits(record.limits);
  const files = uniquePaths(
    expectArray(record, 'largest_files', context, limits.maxResultsPerList).map(parseFile),
    'Storage analysis files',
  );
  const recursiveFolders = uniquePaths(
    expectArray(
      record,
      'largest_folders_by_recursive_bytes',
      context,
      limits.maxResultsPerList,
    ).map((item, index) => parseFolder(item, index, 'recursive')),
    'Storage analysis recursive folders',
  );
  const immediateFolders = uniquePaths(
    expectArray(
      record,
      'largest_folders_by_immediate_bytes',
      context,
      limits.maxResultsPerList,
    ).map((item, index) => parseFolder(item, index, 'immediate')),
    'Storage analysis immediate folders',
  );
  const stopReason = record.stop_reason;
  if (
    stopReason !== null &&
    stopReason !== 'cancelled' &&
    stopReason !== 'duration_limit' &&
    stopReason !== 'entry_limit'
  ) {
    invalid(context, 'stop_reason');
  }
  const result = {
    rootPath: safePath(record, 'root_path', context),
    apparentBytesScanned: u64(record, 'apparent_bytes_scanned', context),
    allocatedBytesScanned: u64(record, 'allocated_bytes_scanned', context),
    truncated: boolean(record, 'truncated', context),
    stopReason: stopReason as StorageAnalysisStopReason | null,
    errors: parseErrors(record.errors),
    skipped: parseSkipped(record.skipped),
    fileResultsOmitted: count(record, 'file_results_omitted', context),
    recursiveFolderResultsOmitted: count(record, 'recursive_folder_results_omitted', context),
    immediateFolderResultsOmitted: count(record, 'immediate_folder_results_omitted', context),
    responseResultsOmitted: count(
      record,
      'response_results_omitted',
      context,
      MAX_RESULTS_PER_LIST * 3,
    ),
    largestFiles: files,
    largestFoldersByRecursiveBytes: recursiveFolders,
    largestFoldersByImmediateBytes: immediateFolders,
    limits,
  };
  const mustBePartial =
    result.stopReason !== null ||
    result.errors.total > 0 ||
    result.skipped.depthLimitedDirectories > 0;
  if (mustBePartial && !result.truncated) {
    throw new ApiError(`${context} marked partial results as complete.`);
  }
  return result;
}

export function parseStorageAnalysisStart(value: unknown): StorageAnalysisStart {
  const context = 'Storage analysis start';
  const record = expectRecord(value, context);
  if (record.state !== 'queued') invalid(context, 'state');
  return { jobId: uuid(record, 'job_id', context), state: 'queued' };
}

export function parseStorageAnalysisStatus(value: unknown): StorageAnalysisStatus {
  const context = 'Storage analysis status';
  const record = expectRecord(value, context);
  const jobState = state(record, context);
  const progress = parseProgress(record.progress);
  const status = {
    jobId: uuid(record, 'job_id', context),
    requestedPath: safePath(record, 'requested_path', context),
    state: jobState,
    progress,
    createdUnixMs: expectNumber(record, 'created_unix_ms', context, {
      integer: true,
      minimum: 0,
      maximum: Number.MAX_SAFE_INTEGER,
    }),
    startedUnixMs: nullableNumber(record, 'started_unix_ms', context),
    finishedUnixMs: nullableNumber(record, 'finished_unix_ms', context),
    cancelRequested: boolean(record, 'cancel_requested', context),
    result: record.result === null ? null : parseResult(record.result),
    error: nullableError(record, context),
  };

  if (
    (status.startedUnixMs !== null && status.startedUnixMs < status.createdUnixMs) ||
    (status.finishedUnixMs !== null &&
      status.finishedUnixMs < (status.startedUnixMs ?? status.createdUnixMs))
  ) {
    throw new ApiError(`${context} returned inconsistent timestamps.`);
  }

  if (jobState === 'queued') {
    if (
      status.startedUnixMs !== null ||
      status.finishedUnixMs !== null ||
      status.result !== null ||
      status.error !== null ||
      progress.percent !== null
    ) {
      throw new ApiError(`${context} returned inconsistent queued state.`);
    }
  } else if (jobState === 'running') {
    if (
      status.startedUnixMs === null ||
      status.finishedUnixMs !== null ||
      status.result !== null ||
      status.error !== null ||
      progress.percent !== null
    ) {
      throw new ApiError(`${context} returned inconsistent running state.`);
    }
  } else if (jobState === 'complete' || jobState === 'cancelled') {
    if (
      status.startedUnixMs === null ||
      status.finishedUnixMs === null ||
      status.result === null ||
      status.error !== null ||
      progress.percent !== 100 ||
      status.result.rootPath !== status.requestedPath ||
      status.result.apparentBytesScanned !== progress.bytesScanned ||
      status.result.allocatedBytesScanned !== progress.allocatedBytesScanned ||
      (jobState === 'cancelled') !== (status.result.stopReason === 'cancelled')
    ) {
      throw new ApiError(`${context} returned inconsistent terminal state.`);
    }
  } else if (
    status.finishedUnixMs === null ||
    status.result !== null ||
    status.error === null
  ) {
    throw new ApiError(`${context} returned inconsistent failed state.`);
  }

  return status;
}

function validatePath(path: string): void {
  const segments = path.split('/');
  if (
    path.length === 0 ||
    !path.startsWith('/') ||
    new TextEncoder().encode(path).length > MAX_PATH_BYTES ||
    hasControlCharacters(path) ||
    path.includes('//') ||
    (path !== '/' && path.endsWith('/')) ||
    segments.some((segment) => segment === '.' || segment === '..')
  ) {
    throw new ApiError('The storage analysis path is invalid.');
  }
}

function validateJobId(jobId: string): void {
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu.test(jobId)) {
    throw new ApiError('The storage analysis job ID is invalid.');
  }
}

export function startStorageAnalysis(
  path: string,
  csrfToken: string,
  mode: StorageAnalysisMode = 'quick',
  signal?: AbortSignal,
): Promise<StorageAnalysisStart> {
  validatePath(path);
  if (mode !== 'quick' && mode !== 'thorough') {
    throw new ApiError('The storage analysis mode is invalid.');
  }
  return requestJson('/api/v1/storage/analysis', parseStorageAnalysisStart, {
    method: 'POST',
    body: { path, mode },
    csrfToken,
    signal,
  });
}

export function getStorageAnalysisStatus(
  jobId: string,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<StorageAnalysisStatus> {
  validateJobId(jobId);
  return requestJson(
    `/api/v1/storage/analysis/${encodeURIComponent(jobId)}`,
    parseStorageAnalysisStatus,
    { csrfToken, signal },
  );
}

export function cancelStorageAnalysis(
  jobId: string,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<StorageAnalysisStatus> {
  validateJobId(jobId);
  return requestJson(
    `/api/v1/storage/analysis/${encodeURIComponent(jobId)}`,
    parseStorageAnalysisStatus,
    { method: 'DELETE', body: {}, csrfToken, signal },
  );
}
