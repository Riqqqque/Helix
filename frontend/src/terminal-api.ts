import { ApiError, expectNumber, expectRecord, expectString, requestJson } from './api';

export interface TerminalStatus {
  availability: 'available' | 'unavailable';
  reauthenticationRequired: true;
  shellPrivilege: 'linux_user';
  persistence: 'while_connected';
  detail: string;
}

export interface TerminalTicket {
  expiresAtUnixMs: number;
  connectPath: '/api/v1/terminal/connect';
  subprotocol: 'helix-terminal-v1';
}

function exactString<T extends string>(
  record: Record<string, unknown>,
  key: string,
  context: string,
  values: readonly T[],
): T {
  const value = expectString(record, key, context);
  if (!values.includes(value as T)) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }
  return value as T;
}

export function parseTerminalStatus(value: unknown): TerminalStatus {
  const context = 'Terminal status';
  const record = expectRecord(value, context);
  if (record.reauthenticationRequired !== true) {
    throw new ApiError('Terminal status returned an invalid reauthentication policy.');
  }
  const detail = expectString(record, 'detail', context);
  if (detail.length > 512 || Array.from(detail).some((character) => /\p{Cc}/u.test(character))) {
    throw new ApiError('Terminal status returned an invalid detail value.');
  }
  return {
    availability: exactString(record, 'availability', context, ['available', 'unavailable'] as const),
    reauthenticationRequired: true,
    shellPrivilege: exactString(record, 'shellPrivilege', context, ['linux_user'] as const),
    persistence: exactString(record, 'persistence', context, ['while_connected'] as const),
    detail,
  };
}

export function parseTerminalTicket(value: unknown): TerminalTicket {
  const context = 'Terminal authorization';
  const record = expectRecord(value, context);
  const connectPath = exactString(record, 'connectPath', context, ['/api/v1/terminal/connect'] as const);
  const subprotocol = exactString(record, 'subprotocol', context, ['helix-terminal-v1'] as const);
  return {
    expiresAtUnixMs: expectNumber(record, 'expiresAtUnixMs', context, {
      integer: true,
      minimum: 1,
      maximum: Number.MAX_SAFE_INTEGER,
    }),
    connectPath,
    subprotocol,
  };
}

export function getTerminalStatus(
  csrfToken: string,
  signal?: AbortSignal,
): Promise<TerminalStatus> {
  return requestJson('/api/v1/terminal/status', parseTerminalStatus, {
    csrfToken,
    signal,
  });
}

export function requestTerminalTicket(
  currentPassword: string,
  columns: number,
  rows: number,
  csrfToken: string,
): Promise<TerminalTicket> {
  return requestJson('/api/v1/terminal/ticket', parseTerminalTicket, {
    method: 'POST',
    body: { currentPassword, columns, rows },
    csrfToken,
  });
}

export function terminalWebSocketUrl(
  connectPath: TerminalTicket['connectPath'],
  locationHref: string,
): string {
  const url = new URL(connectPath, locationHref);
  if (url.protocol === 'http:') url.protocol = 'ws:';
  else if (url.protocol === 'https:') url.protocol = 'wss:';
  else throw new ApiError('Helix cannot open a terminal from this page protocol.');
  return url.href;
}
