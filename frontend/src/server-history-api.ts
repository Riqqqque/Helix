import { ApiError, expectArray, expectNumber, expectRecord, expectString, requestJson } from './api';

export type ConsoleHistoryKind = 'boot' | 'command' | 'command_response' | 'output';

export interface ConsoleHistoryEntry {
  kind: ConsoleHistoryKind;
  text: string;
  timestamp: string | null;
  timestampUnixMs: number | null;
  bootStartedAt: string | null;
}

export interface ConsoleHistoryPage {
  instanceId: string;
  entries: ConsoleHistoryEntry[];
  nextCursor: string | null;
  hasMore: boolean;
  order: 'chronological_within_page';
  paginationDirection: 'newest_to_older';
  pageTextByteLimit: number;
  retention: { maximumBytes: number; files: number; scope: 'per_server' };
  collectedAtUnixMs: number;
}

function exact<T extends string>(value: unknown, options: readonly T[], context: string): T {
  if (typeof value !== 'string' || !options.includes(value as T)) throw new ApiError(`${context} returned an invalid value.`);
  return value as T;
}

function optionalText(value: unknown, context: string, maximumBytes: number): string | null {
  if (value === null) return null;
  if (typeof value !== 'string' || new TextEncoder().encode(value).length > maximumBytes) {
    throw new ApiError(`${context} returned an invalid value.`);
  }
  return value;
}

function optionalCount(value: unknown, context: string): number | null {
  if (value === null) return null;
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    throw new ApiError(`${context} returned an invalid value.`);
  }
  return value;
}

export function parseConsoleHistoryPage(value: unknown): ConsoleHistoryPage {
  const root = expectRecord(value, 'console history');
  if (expectNumber(root, 'schema_version', 'console history', { integer: true }) !== 1) {
    throw new ApiError('Console history returned an unsupported schema.');
  }
  const pageTextByteLimit = expectNumber(root, 'page_text_byte_limit', 'console history', { integer: true, minimum: 1, maximum: 4 * 1024 * 1024 });
  let textBytes = 0;
  const entries = expectArray(root, 'entries', 'console history', 500).map((value) => {
    const item = expectRecord(value, 'console history entry');
    const text = optionalText(item.text, 'Console history text', pageTextByteLimit);
    if (text === null) throw new ApiError('Console history text cannot be null.');
    textBytes += new TextEncoder().encode(text).length;
    return {
      kind: exact(item.kind, ['boot', 'command', 'command_response', 'output'] as const, 'Console history kind'),
      text,
      timestamp: optionalText(item.timestamp, 'Console history timestamp', 128),
      timestampUnixMs: optionalCount(item.timestamp_unix_ms, 'Console history timestamp'),
      bootStartedAt: optionalText(item.boot_started_at, 'Console history boot timestamp', 64),
    };
  });
  if (textBytes > pageTextByteLimit) throw new ApiError('Console history exceeded its declared page limit.');
  const nextCursorValue = root.next_cursor;
  const nextCursor = nextCursorValue === null ? null : optionalText(nextCursorValue, 'Console history cursor', 64);
  if (nextCursor !== null && !/^h1\.[0-9a-f]{16}\.[0-9a-f]{16}$/u.test(nextCursor)) {
    throw new ApiError('Console history returned an invalid cursor.');
  }
  if (typeof root.has_more !== 'boolean' || root.has_more !== (nextCursor !== null)) {
    throw new ApiError('Console history returned inconsistent pagination state.');
  }
  const retention = expectRecord(root.retention, 'console history retention');
  return {
    instanceId: expectString(root, 'instance_id', 'console history'),
    entries,
    nextCursor,
    hasMore: root.has_more,
    order: exact(root.order, ['chronological_within_page'] as const, 'Console history order'),
    paginationDirection: exact(root.pagination_direction, ['newest_to_older'] as const, 'Console history pagination direction'),
    pageTextByteLimit,
    retention: {
      maximumBytes: expectNumber(retention, 'maximum_bytes', 'console history retention', { integer: true, minimum: 1 }),
      files: expectNumber(retention, 'files', 'console history retention', { integer: true, minimum: 1 }),
      scope: exact(retention.scope, ['per_server'] as const, 'Console history retention scope'),
    },
    collectedAtUnixMs: expectNumber(root, 'collected_at_unix_ms', 'console history', { integer: true, minimum: 0 }),
  };
}

export function getServerLogHistory(
  instanceId: string,
  csrfToken: string,
  cursor: string | null = null,
  lines = 200,
  signal?: AbortSignal,
): Promise<ConsoleHistoryPage> {
  const query = new URLSearchParams({ lines: String(lines) });
  if (cursor !== null) query.set('cursor', cursor);
  return requestJson(
    `/api/v1/servers/${encodeURIComponent(instanceId)}/logs/history?${query.toString()}`,
    parseConsoleHistoryPage,
    { csrfToken, signal, timeoutMs: 25_000 },
  );
}

export function consoleHistoryEntryKey(entry: ConsoleHistoryEntry): string {
  return JSON.stringify([entry.kind, entry.text, entry.timestamp, entry.timestampUnixMs, entry.bootStartedAt]);
}

function overlapLength(left: readonly ConsoleHistoryEntry[], right: readonly ConsoleHistoryEntry[]): number {
  const maximum = Math.min(left.length, right.length);
  for (let length = maximum; length > 0; length -= 1) {
    let matches = true;
    for (let index = 0; index < length; index += 1) {
      if (consoleHistoryEntryKey(left[left.length - length + index]!) !== consoleHistoryEntryKey(right[index]!)) {
        matches = false;
        break;
      }
    }
    if (matches) return length;
  }
  return 0;
}

export function mergeLatestConsoleHistory(
  current: readonly ConsoleHistoryEntry[],
  latest: readonly ConsoleHistoryEntry[],
): ConsoleHistoryEntry[] {
  if (current.length === 0) return [...latest];
  if (latest.length === 0) return [...current];
  const overlap = overlapLength(current, latest);
  return [...current, ...latest.slice(overlap)];
}

export function prependOlderConsoleHistory(
  current: readonly ConsoleHistoryEntry[],
  older: readonly ConsoleHistoryEntry[],
): ConsoleHistoryEntry[] {
  if (older.length === 0) return [...current];
  if (current.length === 0) return [...older];
  const overlap = overlapLength(older, current);
  return [...older.slice(0, older.length - overlap), ...current];
}
