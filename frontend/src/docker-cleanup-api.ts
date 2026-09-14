import { ApiError, expectArray, expectNumber, expectRecord, expectString, requestJson } from './api';
import type { BrokerJob } from './control-api';
import type { RebootWeekday } from './host-api';

export interface DockerCleanupStorage {
  dataRoot: string;
  storageDriver: string;
  mountPoint: string;
  source: string;
  totalBytes: number;
  availableBytes: number;
}

export interface DockerUsageCategory {
  kind: 'images' | 'containers' | 'local_volumes' | 'build_cache';
  totalCount: number;
  activeCount: number;
  sizeBytes: number;
  reclaimableBytes: number;
}

export type DockerCleanupSchedule =
  | { state: 'none' }
  | { state: 'unavailable'; reason: string }
  | {
      state: 'scheduled' | 'degraded';
      scheduleId: string;
      weekdays: RebootWeekday[];
      hour: number;
      minute: number;
      timezone: string;
      retentionHours: number;
      calendarExpression: string;
      nextAtUnixMs: number | null;
      timerActive: boolean;
      timerEnabled: boolean;
    };

export interface DockerCleanupLastRun {
  status: 'running' | 'complete' | 'failed' | 'unavailable';
  trigger?: 'manual' | 'scheduled';
  retentionHours?: number;
  startedAtUnixMs?: number;
  finishedAtUnixMs?: number | null;
  reclaimableBeforeBytes?: number;
  availableBeforeBytes?: number;
  availableAfterBytes?: number | null;
  completedSteps?: string[];
  error?: string | null;
}

export interface DockerCleanupStatus {
  availability: 'ready' | 'unavailable';
  dockerInstalled: boolean;
  storage: DockerCleanupStorage | null;
  usage: DockerUsageCategory[];
  reclaimableBytes: number;
  defaultRetentionHours: number;
  minimumRetentionHours: number;
  maximumRetentionHours: number;
  schedule: DockerCleanupSchedule;
  lastRun: DockerCleanupLastRun | null;
  activeJob: BrokerJob | null;
  error: string | null;
  collectedAtUnixMs: number;
}

export interface DockerCleanupScheduleInput {
  weekdays: RebootWeekday[];
  hour: number;
  minute: number;
  timezone: string;
  retentionHours: number;
}

const weekdays = new Set<RebootWeekday>([
  'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday',
]);
const usageKinds = new Set<DockerUsageCategory['kind']>([
  'images', 'containers', 'local_volumes', 'build_cache',
]);
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;

function boolean(record: Record<string, unknown>, key: string, context: string): boolean {
  const value = record[key];
  if (typeof value !== 'boolean') throw new ApiError(`${context} returned an invalid ${key} value.`);
  return value;
}

function optionalString(record: Record<string, unknown>, key: string, maximum = 500): string | null {
  const value = record[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'string' || value.length > maximum || Array.from(value).some((character) => /\p{Cc}/u.test(character))) {
    throw new ApiError(`Docker cleanup returned an invalid ${key} value.`);
  }
  return value;
}

function optionalNumber(record: Record<string, unknown>, key: string): number | null {
  const value = record[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    throw new ApiError(`Docker cleanup returned an invalid ${key} value.`);
  }
  return value;
}

function parseWeekdays(value: unknown): RebootWeekday[] {
  if (!Array.isArray(value) || value.length === 0 || value.length > 7) {
    throw new ApiError('Docker cleanup returned an invalid weekdays value.');
  }
  return value.map((entry) => {
    if (typeof entry !== 'string' || !weekdays.has(entry as RebootWeekday)) {
      throw new ApiError('Docker cleanup returned an invalid weekday.');
    }
    return entry as RebootWeekday;
  });
}

function parseJob(value: unknown): BrokerJob {
  const root = expectRecord(value, 'Docker cleanup job');
  const status = expectString(root, 'status', 'Docker cleanup job');
  if (!['queued', 'running', 'complete', 'failed'].includes(status)) {
    throw new ApiError('Docker cleanup returned an invalid job status.');
  }
  const id = expectString(root, 'id', 'Docker cleanup job');
  if (!uuid.test(id)) throw new ApiError('Docker cleanup returned an invalid job ID.');
  return {
    id,
    kind: expectString(root, 'kind', 'Docker cleanup job'),
    status: status as BrokerJob['status'],
    stage: expectString(root, 'stage', 'Docker cleanup job'),
    progressPercent: expectNumber(root, 'progress_percent', 'Docker cleanup job', { integer: true, minimum: 0, maximum: 100 }),
    createdAtUnixMs: expectNumber(root, 'created_at_unix_ms', 'Docker cleanup job', { integer: true, minimum: 0 }),
    updatedAtUnixMs: expectNumber(root, 'updated_at_unix_ms', 'Docker cleanup job', { integer: true, minimum: 0 }),
    result: root.result ?? null,
    error: optionalString(root, 'error'),
  };
}

function parseSchedule(value: unknown): DockerCleanupSchedule {
  const root = expectRecord(value, 'Docker cleanup schedule');
  const state = expectString(root, 'state', 'Docker cleanup schedule');
  if (state === 'none') return { state };
  if (state === 'unavailable') {
    return { state, reason: expectString(root, 'reason', 'Docker cleanup schedule') };
  }
  if (state !== 'scheduled' && state !== 'degraded') {
    throw new ApiError('Docker cleanup returned an invalid schedule state.');
  }
  const scheduleId = expectString(root, 'schedule_id', 'Docker cleanup schedule');
  if (!uuid.test(scheduleId)) throw new ApiError('Docker cleanup returned an invalid schedule ID.');
  const retentionHours = expectNumber(root, 'retention_hours', 'Docker cleanup schedule', { integer: true, minimum: 24, maximum: 8_760 });
  return {
    state,
    scheduleId,
    weekdays: parseWeekdays(root.weekdays),
    hour: expectNumber(root, 'hour', 'Docker cleanup schedule', { integer: true, minimum: 0, maximum: 23 }),
    minute: expectNumber(root, 'minute', 'Docker cleanup schedule', { integer: true, minimum: 0, maximum: 59 }),
    timezone: expectString(root, 'timezone', 'Docker cleanup schedule'),
    retentionHours,
    calendarExpression: expectString(root, 'calendar_expression', 'Docker cleanup schedule'),
    nextAtUnixMs: optionalNumber(root, 'next_at_unix_ms'),
    timerActive: boolean(root, 'timer_active', 'Docker cleanup schedule'),
    timerEnabled: boolean(root, 'timer_enabled', 'Docker cleanup schedule'),
  };
}

function parseLastRun(value: unknown): DockerCleanupLastRun | null {
  if (value === null || value === undefined) return null;
  const root = expectRecord(value, 'Docker cleanup history');
  const status = expectString(root, 'status', 'Docker cleanup history');
  if (status === 'unavailable') return { status, error: optionalString(root, 'error') };
  if (status !== 'running' && status !== 'complete' && status !== 'failed') {
    throw new ApiError('Docker cleanup returned an invalid history state.');
  }
  const trigger = expectString(root, 'trigger', 'Docker cleanup history');
  if (trigger !== 'manual' && trigger !== 'scheduled') {
    throw new ApiError('Docker cleanup returned an invalid history trigger.');
  }
  return {
    status,
    trigger,
    retentionHours: expectNumber(root, 'retention_hours', 'Docker cleanup history', { integer: true, minimum: 24, maximum: 8_760 }),
    startedAtUnixMs: expectNumber(root, 'started_at_unix_ms', 'Docker cleanup history', { integer: true, minimum: 0 }),
    finishedAtUnixMs: optionalNumber(root, 'finished_at_unix_ms'),
    reclaimableBeforeBytes: expectNumber(root, 'reclaimable_before_bytes', 'Docker cleanup history', { integer: true, minimum: 0 }),
    availableBeforeBytes: expectNumber(root, 'available_before_bytes', 'Docker cleanup history', { integer: true, minimum: 0 }),
    availableAfterBytes: optionalNumber(root, 'available_after_bytes'),
    completedSteps: expectArray(root, 'completed_steps', 'Docker cleanup history', 3).map((step) => {
      if (typeof step !== 'string') throw new ApiError('Docker cleanup returned an invalid completed step.');
      return step;
    }),
    error: optionalString(root, 'error'),
  };
}

export function parseDockerCleanupStatus(value: unknown): DockerCleanupStatus {
  const root = expectRecord(value, 'Docker cleanup');
  if (expectNumber(root, 'schema_version', 'Docker cleanup', { integer: true, minimum: 1, maximum: 1 }) !== 1) {
    throw new ApiError('Docker cleanup returned an unsupported schema.');
  }
  const availability = expectString(root, 'availability', 'Docker cleanup');
  if (availability !== 'ready' && availability !== 'unavailable') {
    throw new ApiError('Docker cleanup returned an invalid availability state.');
  }
  const policy = expectRecord(root.policy, 'Docker cleanup policy');
  const storage = root.storage === null ? null : (() => {
    const item = expectRecord(root.storage, 'Docker cleanup storage');
    return {
      dataRoot: expectString(item, 'data_root', 'Docker cleanup storage'),
      storageDriver: expectString(item, 'storage_driver', 'Docker cleanup storage'),
      mountPoint: expectString(item, 'mount_point', 'Docker cleanup storage'),
      source: expectString(item, 'source', 'Docker cleanup storage'),
      totalBytes: expectNumber(item, 'total_bytes', 'Docker cleanup storage', { integer: true, minimum: 0 }),
      availableBytes: expectNumber(item, 'available_bytes', 'Docker cleanup storage', { integer: true, minimum: 0 }),
    };
  })();
  const usage = expectArray(root, 'usage', 'Docker cleanup', 8).map((entry) => {
    const item = expectRecord(entry, 'Docker usage category');
    const kind = expectString(item, 'kind', 'Docker usage category') as DockerUsageCategory['kind'];
    if (!usageKinds.has(kind)) throw new ApiError('Docker cleanup returned an unknown usage category.');
    return {
      kind,
      totalCount: expectNumber(item, 'total_count', 'Docker usage category', { integer: true, minimum: 0 }),
      activeCount: expectNumber(item, 'active_count', 'Docker usage category', { integer: true, minimum: 0 }),
      sizeBytes: expectNumber(item, 'size_bytes', 'Docker usage category', { integer: true, minimum: 0 }),
      reclaimableBytes: expectNumber(item, 'reclaimable_bytes', 'Docker usage category', { integer: true, minimum: 0 }),
    };
  });
  return {
    availability,
    dockerInstalled: boolean(root, 'docker_installed', 'Docker cleanup'),
    storage,
    usage,
    reclaimableBytes: expectNumber(root, 'reclaimable_bytes', 'Docker cleanup', { integer: true, minimum: 0 }),
    defaultRetentionHours: expectNumber(policy, 'default_retention_hours', 'Docker cleanup policy', { integer: true, minimum: 24, maximum: 8_760 }),
    minimumRetentionHours: expectNumber(policy, 'minimum_retention_hours', 'Docker cleanup policy', { integer: true, minimum: 24, maximum: 8_760 }),
    maximumRetentionHours: expectNumber(policy, 'maximum_retention_hours', 'Docker cleanup policy', { integer: true, minimum: 24, maximum: 8_760 }),
    schedule: parseSchedule(root.schedule),
    lastRun: parseLastRun(root.last_run),
    activeJob: root.active_job === null || root.active_job === undefined ? null : parseJob(root.active_job),
    error: optionalString(root, 'error'),
    collectedAtUnixMs: expectNumber(root, 'collected_at_unix_ms', 'Docker cleanup', { integer: true, minimum: 0 }),
  };
}

export function getDockerCleanupStatus(csrfToken: string, signal?: AbortSignal): Promise<DockerCleanupStatus> {
  return requestJson('/api/v1/docker/cleanup', parseDockerCleanupStatus, { csrfToken, signal, timeoutMs: 40_000 });
}

export function startDockerCleanup(retentionHours: number, csrfToken: string): Promise<{ jobId: string; reused: boolean }> {
  return requestJson('/api/v1/docker/cleanup/run', (value) => {
    const root = expectRecord(value, 'Docker cleanup dispatch');
    const jobId = expectString(root, 'job_id', 'Docker cleanup dispatch');
    if (!uuid.test(jobId)) throw new ApiError('Docker cleanup returned an invalid job ID.');
    return { jobId, reused: boolean(root, 'reused', 'Docker cleanup dispatch') };
  }, { method: 'POST', csrfToken, body: { retention_hours: retentionHours } });
}

export function getDockerCleanupJob(jobId: string, csrfToken: string, signal?: AbortSignal): Promise<BrokerJob> {
  if (!uuid.test(jobId)) return Promise.reject(new ApiError('That Docker cleanup job ID is invalid.'));
  return requestJson(`/api/v1/docker/cleanup/jobs/${encodeURIComponent(jobId)}`, parseJob, { csrfToken, signal });
}

export function setDockerCleanupSchedule(input: DockerCleanupScheduleInput, csrfToken: string): Promise<DockerCleanupSchedule> {
  return requestJson('/api/v1/docker/cleanup/schedule', parseSchedule, {
    method: 'PUT',
    csrfToken,
    body: {
      weekdays: input.weekdays,
      hour: input.hour,
      minute: input.minute,
      timezone: input.timezone,
      retention_hours: input.retentionHours,
    },
  });
}

export function deleteDockerCleanupSchedule(csrfToken: string): Promise<void> {
  return requestJson('/api/v1/docker/cleanup/schedule', () => undefined, { method: 'DELETE', csrfToken });
}
