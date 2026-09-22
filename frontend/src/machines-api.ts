import {
  ApiError,
  expectArray,
  expectBoolean,
  expectNumber,
  expectRecord,
  expectString,
  requestJson,
  type JsonRecord,
} from './api';

export type MachineAuthKind = 'key' | 'password' | 'system';
export type MachineProbeStatus =
  | 'online'
  | 'reachable'
  | 'unreachable'
  | 'auth_failed'
  | 'host_key'
  | 'error';
export type MachinePowerAction = 'reboot' | 'poweroff';

export interface MachineProbe {
  status: MachineProbeStatus;
  detail: string;
  latencyMs: number | null;
  hostname: string | null;
  os: string | null;
  kernel: string | null;
  uptimeSeconds: number | null;
  load: [number, number, number] | null;
  cpuCount: number | null;
  memTotalBytes: number | null;
  memAvailableBytes: number | null;
  diskTotalBytes: number | null;
  diskAvailableBytes: number | null;
  containersRunning: number | null;
  failedUnits: number | null;
}

export interface Machine {
  id: string;
  label: string;
  host: string;
  port: number;
  username: string;
  authKind: MachineAuthKind;
  notes: string;
  wolMac: string | null;
  probe: MachineProbe | null;
  probedAtUnixMs: number | null;
  createdAtUnixMs: number;
  updatedAtUnixMs: number;
}

export interface MachineUpsert {
  label: string;
  host: string;
  port: number;
  username: string;
  authKind: MachineAuthKind;
  notes: string;
  wolMac: string | null;
}

export interface HubIdentity {
  available: boolean;
  publicKey: string | null;
  fingerprintSha256: string | null;
  user: string | null;
  detail: string;
}

export interface MachineTicket {
  expiresAtUnixMs: number;
  connectPath: string;
  subprotocol: 'helix-terminal-v1';
}

const MAX_MACHINES = 64;
const MAX_TEXT_BYTES = 1_024;

function boundedText(value: unknown, maximum = MAX_TEXT_BYTES): string | null {
  if (typeof value !== 'string' || new TextEncoder().encode(value).length > maximum) return null;
  if (Array.from(value).some((character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint <= 0x1f || codePoint === 0x7f;
  })) return null;
  return value;
}

function optionalText(record: JsonRecord, key: string, maximum = MAX_TEXT_BYTES): string | null {
  if (record[key] === null || record[key] === undefined) return null;
  const value = boundedText(record[key], maximum);
  if (value === null) throw new ApiError(`Machine data returned an invalid ${key} value.`);
  return value;
}

function optionalNumber(
  record: JsonRecord,
  key: string,
  minimum = 0,
  maximum = Number.MAX_SAFE_INTEGER,
): number | null {
  if (record[key] === null || record[key] === undefined) return null;
  const value = record[key];
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < minimum || value > maximum) {
    throw new ApiError(`Machine data returned an invalid ${key} value.`);
  }
  return value;
}

export function parseMachineProbe(value: unknown): MachineProbe {
  const context = 'Machine probe';
  const record = expectRecord(value, context);
  const status = expectString(record, 'status', context);
  if (
    status !== 'online' &&
    status !== 'reachable' &&
    status !== 'unreachable' &&
    status !== 'auth_failed' &&
    status !== 'host_key' &&
    status !== 'error'
  ) {
    throw new ApiError(`${context} returned an invalid status value.`);
  }
  const detail = optionalText(record, 'detail') ?? '';
  const loadValue = record.load;
  let load: [number, number, number] | null = null;
  if (loadValue !== null && loadValue !== undefined) {
    if (
      !Array.isArray(loadValue) ||
      loadValue.length !== 3 ||
      !loadValue.every((entry) => typeof entry === 'number' && Number.isFinite(entry) && entry >= 0 && entry <= 1_000_000)
    ) {
      throw new ApiError(`${context} returned an invalid load value.`);
    }
    load = [loadValue[0], loadValue[1], loadValue[2]];
  }
  return {
    status,
    detail,
    latencyMs: optionalNumber(record, 'latencyMs', 0, 86_400_000),
    hostname: optionalText(record, 'hostname', 128),
    os: optionalText(record, 'os', 128),
    kernel: optionalText(record, 'kernel', 128),
    uptimeSeconds: optionalNumber(record, 'uptimeSeconds', 0, 4_000_000_000),
    load,
    cpuCount: optionalNumber(record, 'cpuCount', 1, 65_535),
    memTotalBytes: optionalNumber(record, 'memTotalBytes'),
    memAvailableBytes: optionalNumber(record, 'memAvailableBytes'),
    diskTotalBytes: optionalNumber(record, 'diskTotalBytes'),
    diskAvailableBytes: optionalNumber(record, 'diskAvailableBytes'),
    containersRunning: optionalNumber(record, 'containersRunning', 0, 100_000),
    failedUnits: optionalNumber(record, 'failedUnits', 0, 100_000),
  };
}

export function parseMachine(value: unknown): Machine {
  const context = 'Machine';
  const record = expectRecord(value, context);
  const id = expectString(record, 'id', context);
  const label = expectString(record, 'label', context);
  const host = expectString(record, 'host', context);
  const username = expectString(record, 'username', context);
  if (id.length > 64 || label.length > 48 || host.length > 253 || username.length > 64) {
    throw new ApiError(`${context} returned out-of-range text.`);
  }
  const authKind = expectString(record, 'authKind', context);
  if (authKind !== 'key' && authKind !== 'password' && authKind !== 'system') {
    throw new ApiError(`${context} returned an invalid authKind value.`);
  }
  const probe = record.probe === null || record.probe === undefined
    ? null
    : parseMachineProbe(record.probe);
  const notes = record.notes === null || record.notes === undefined
    ? ''
    : optionalText(record, 'notes', 2_048) ?? '';
  return {
    id,
    label,
    host,
    port: expectNumber(record, 'port', context, { integer: true, minimum: 1, maximum: 65_535 }),
    username,
    authKind,
    notes,
    wolMac: optionalText(record, 'wolMac', 32),
    probe,
    probedAtUnixMs: optionalNumber(record, 'probedAtUnixMs'),
    createdAtUnixMs: expectNumber(record, 'createdAtUnixMs', context, { integer: true, minimum: 0 }),
    updatedAtUnixMs: expectNumber(record, 'updatedAtUnixMs', context, { integer: true, minimum: 0 }),
  };
}

function parseMachineList(value: unknown): Machine[] {
  const record = expectRecord(value, 'Machine list');
  return expectArray(record, 'machines', 'Machine list', MAX_MACHINES).map(parseMachine);
}

function parseCreatedMachine(value: unknown): Machine {
  return parseMachine(expectRecord(value, 'Machine result').machine);
}

export function parseHubIdentity(value: unknown): HubIdentity {
  const context = 'Hub identity';
  const record = expectRecord(value, context);
  return {
    available: expectBoolean(record, 'available', context),
    publicKey: optionalText(record, 'publicKey', 512),
    fingerprintSha256: optionalText(record, 'fingerprintSha256', 128),
    user: optionalText(record, 'user', 64),
    detail: optionalText(record, 'detail', 512) ?? '',
  };
}

function parseMachineTicket(value: unknown): MachineTicket {
  const context = 'Machine terminal authorization';
  const record = expectRecord(value, context);
  const connectPath = expectString(record, 'connectPath', context);
  if (
    connectPath.length > 128 ||
    !connectPath.startsWith('/api/v1/machines/') ||
    !connectPath.endsWith('/terminal/connect')
  ) {
    throw new ApiError(`${context} returned an invalid connectPath value.`);
  }
  const subprotocol = expectString(record, 'subprotocol', context);
  if (subprotocol !== 'helix-terminal-v1') {
    throw new ApiError(`${context} returned an invalid subprotocol value.`);
  }
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

function parseRefreshResults(value: unknown): Array<{ id: string; probe: MachineProbe }> {
  const record = expectRecord(value, 'Machine refresh');
  return expectArray(record, 'probes', 'Machine refresh', MAX_MACHINES).map((entry) => {
    const item = expectRecord(entry, 'Machine refresh');
    return { id: expectString(item, 'id', 'Machine refresh'), probe: parseMachineProbe(item.probe) };
  });
}

function parseDetail(value: unknown): { detail: string } {
  const record = expectRecord(value, 'Machine action');
  return { detail: optionalText(record, 'detail', 512) ?? '' };
}

export function listMachines(csrfToken: string, signal?: AbortSignal): Promise<Machine[]> {
  return requestJson('/api/v1/machines', parseMachineList, { csrfToken, signal });
}

export function createMachine(input: MachineUpsert, csrfToken: string): Promise<Machine> {
  return requestJson('/api/v1/machines', parseCreatedMachine, {
    method: 'POST',
    body: input,
    csrfToken,
  });
}

export function updateMachine(
  machineId: string,
  input: MachineUpsert,
  csrfToken: string,
): Promise<Machine> {
  return requestJson(`/api/v1/machines/${encodeURIComponent(machineId)}`, parseCreatedMachine, {
    method: 'PUT',
    body: input,
    csrfToken,
  });
}

export function deleteMachine(machineId: string, csrfToken: string): Promise<void> {
  return requestJson(`/api/v1/machines/${encodeURIComponent(machineId)}`, () => undefined, {
    method: 'DELETE',
    body: {},
    csrfToken,
  });
}

export function getHubIdentity(csrfToken: string, signal?: AbortSignal): Promise<HubIdentity> {
  return requestJson('/api/v1/machines/identity', parseHubIdentity, { csrfToken, signal });
}

export function probeMachine(machineId: string, csrfToken: string): Promise<MachineProbe> {
  return requestJson(`/api/v1/machines/${encodeURIComponent(machineId)}/probe`, parseMachineProbe, {
    method: 'POST',
    body: {},
    csrfToken,
    timeoutMs: 30_000,
  });
}

export function refreshMachines(
  csrfToken: string,
): Promise<Array<{ id: string; probe: MachineProbe }>> {
  return requestJson('/api/v1/machines/refresh', parseRefreshResults, {
    method: 'POST',
    body: {},
    csrfToken,
    timeoutMs: 60_000,
  });
}

export function wakeMachine(machineId: string, csrfToken: string): Promise<{ detail: string }> {
  return requestJson(`/api/v1/machines/${encodeURIComponent(machineId)}/wake`, parseDetail, {
    method: 'POST',
    body: {},
    csrfToken,
    timeoutMs: 15_000,
  });
}

export function powerMachine(
  machineId: string,
  action: MachinePowerAction,
  confirmation: string,
  csrfToken: string,
): Promise<{ detail: string }> {
  return requestJson(`/api/v1/machines/${encodeURIComponent(machineId)}/power`, parseDetail, {
    method: 'POST',
    body: { action, confirmation },
    csrfToken,
    timeoutMs: 30_000,
  });
}

export function requestMachineTicket(
  machineId: string,
  currentPassword: string,
  columns: number,
  rows: number,
  csrfToken: string,
): Promise<MachineTicket> {
  return requestJson(
    `/api/v1/machines/${encodeURIComponent(machineId)}/terminal/ticket`,
    parseMachineTicket,
    {
      method: 'POST',
      body: { currentPassword, columns, rows },
      csrfToken,
    },
  );
}

export function machineWebSocketUrl(connectPath: string, locationHref: string): string {
  const url = new URL(connectPath, locationHref);
  if (url.protocol === 'http:') url.protocol = 'ws:';
  else if (url.protocol === 'https:') url.protocol = 'wss:';
  else throw new ApiError('Helix cannot open a terminal from this page protocol.');
  return url.href;
}
