import type {
  AccountUpdateInput,
  AuthenticatedUser,
  AuthenticatedUserResponse,
  AuthSession,
  CsrfRotation,
  HealthSnapshot,
  LoginInput,
  NetworkAddressSnapshot,
  NetworkInterfaceSnapshot,
  NetworkSnapshot,
  OwnerSetupInput,
  SetupStatus,
  StorageMountSnapshot,
  StorageSnapshot,
  SystemOverview,
} from './types';
import {
  beginMutationRequest,
  endMutationRequest,
  rememberOperationDispatch,
} from './operation-continuity';

const REQUEST_TIMEOUT_MS = 8_000;
const MAX_STORAGE_MOUNTS = 64;
const MAX_NETWORK_INTERFACES = 64;
const MAX_ADDRESSES_PER_INTERFACE = 16;
const MAX_NETWORK_ADDRESSES = 256;
const MAX_HOST_TEXT_BYTES = 32_768;
const U64_MAX = 18_446_744_073_709_551_615n;

export type JsonRecord = Record<string, unknown>;

export class ApiError extends Error {
  override readonly name = 'ApiError';

  constructor(
    message: string,
    readonly status: number | null = null,
    readonly code: string | null = null,
  ) {
    super(message);
  }
}

export function expectRecord(value: unknown, context: string): JsonRecord {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new ApiError(`${context} returned an invalid response.`);
  }

  return value as JsonRecord;
}

export function expectString(record: JsonRecord, key: string, context: string): string {
  const value = record[key];
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }

  return value;
}

export function expectBoolean(record: JsonRecord, key: string, context: string): boolean {
  const value = record[key];
  if (typeof value !== 'boolean') {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }

  return value;
}

export function expectArray(
  record: JsonRecord,
  key: string,
  context: string,
  maximumLength: number,
): unknown[] {
  const value = record[key];
  if (!Array.isArray(value) || value.length > maximumLength) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }

  return value;
}

function expectHostText(
  record: JsonRecord,
  key: string,
  context: string,
  maximumBytes = MAX_HOST_TEXT_BYTES,
): string {
  const value = record[key];
  if (
    typeof value !== 'string' ||
    value.trim().length === 0 ||
    new TextEncoder().encode(value).length > maximumBytes ||
    Array.from(value).some((character) => {
      const codePoint = character.codePointAt(0) ?? 0;
      return codePoint <= 0x1f || codePoint === 0x7f;
    })
  ) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }

  return value;
}

function expectNullableHostText(
  record: JsonRecord,
  key: string,
  context: string,
): string | null {
  if (record[key] === null) {
    return null;
  }

  return expectHostText(record, key, context);
}

export function expectNumber(
  record: JsonRecord,
  key: string,
  context: string,
  options: { integer?: boolean; minimum?: number; maximum?: number } = {},
): number {
  const value = record[key];
  const isInteger = options.integer === true ? Number.isSafeInteger(value) : true;
  const aboveMinimum =
    options.minimum === undefined ||
    (typeof value === 'number' && value >= options.minimum);
  const belowMaximum =
    options.maximum === undefined ||
    (typeof value === 'number' && value <= options.maximum);

  if (
    typeof value !== 'number' ||
    !Number.isFinite(value) ||
    !isInteger ||
    !aboveMinimum ||
    !belowMaximum
  ) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }

  return value;
}

function expectNullablePercent(
  record: JsonRecord,
  key: string,
  context: string,
): number | null {
  if (record[key] === null) {
    return null;
  }

  return expectNumber(record, key, context, { minimum: 0, maximum: 100 });
}

function expectNullableNumber(
  record: JsonRecord,
  key: string,
  context: string,
  options: { integer?: boolean; minimum?: number; maximum?: number } = {},
): number | null {
  if (record[key] === null) {
    return null;
  }

  return expectNumber(record, key, context, options);
}

function expectU64String(record: JsonRecord, key: string, context: string): bigint {
  const value = record[key];
  if (
    typeof value !== 'string' ||
    !/^(?:0|[1-9]\d*)$/u.test(value) ||
    value.length > 20
  ) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }

  const parsed = BigInt(value);
  if (parsed > U64_MAX) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }

  return parsed;
}

function expectDiscoveryAvailability(
  record: JsonRecord,
  context: string,
): 'available' | 'degraded' | 'unavailable' {
  const value = record.availability;
  if (value !== 'available' && value !== 'degraded' && value !== 'unavailable') {
    throw new ApiError(`${context} returned an invalid availability value.`);
  }

  return value;
}

function expectStringArray(record: JsonRecord, key: string, context: string): string[] {
  const value = record[key];
  if (
    !Array.isArray(value) ||
    value.length > 256 ||
    value.some(
      (item) =>
        typeof item !== 'string' ||
        item.length === 0 ||
        item.length > 128 ||
        !/^[a-z0-9]+(?:\.[a-z0-9]+)*$/.test(item),
    ) ||
    new Set(value).size !== value.length
  ) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }

  return value as string[];
}

function expectOpaqueToken(record: JsonRecord, key: string, context: string): string {
  const value = expectString(record, key, context);
  if (!/^[A-Za-z0-9_-]{43}$/.test(value)) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }
  return value;
}

function parseUser(value: unknown, context: string): AuthenticatedUser {
  const record = expectRecord(value, context);
  const id = expectString(record, 'id', context);
  const loginName = expectString(record, 'loginName', context);
  const displayName = expectString(record, 'displayName', context);
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(id)) {
    throw new ApiError(`${context} returned an invalid id value.`);
  }
  if (!/^[a-z0-9](?:[a-z0-9._-]{1,62}[a-z0-9])$/.test(loginName)) {
    throw new ApiError(`${context} returned an invalid loginName value.`);
  }
  if (
    displayName !== displayName.normalize('NFC') ||
    displayName.trim() !== displayName ||
    displayName.length === 0 ||
    Array.from(displayName).length > 128 ||
    new TextEncoder().encode(displayName).length > 512 ||
    Array.from(displayName).some((character) => /\p{Cc}/u.test(character))
  ) {
    throw new ApiError(`${context} returned an invalid displayName value.`);
  }
  return {
    id,
    loginName,
    displayName,
    capabilities: expectStringArray(record, 'capabilities', context),
  };
}

export function parseSetupStatus(value: unknown): SetupStatus {
  const context = 'Setup status API';
  const record = expectRecord(value, context);
  return {
    ownerExists: expectBoolean(record, 'ownerExists', context),
    bootstrapAvailable: expectBoolean(record, 'bootstrapAvailable', context),
    bootstrapExpiresAtUnixMs: expectNullableNumber(
      record,
      'bootstrapExpiresAtUnixMs',
      context,
      { integer: true, minimum: 0 },
    ),
  };
}

export function parseAuthSession(value: unknown): AuthSession {
  const context = 'Authentication API';
  const record = expectRecord(value, context);
  return {
    user: parseUser(record.user, `${context} user`),
    csrfToken: expectOpaqueToken(record, 'csrfToken', context),
    expiresAtUnixMs: expectNumber(record, 'expiresAtUnixMs', context, {
      integer: true,
      minimum: 0,
    }),
    sessionExpires: expectBoolean(record, 'sessionExpires', context),
  };
}

export function parseAuthenticatedUserResponse(
  value: unknown,
): AuthenticatedUserResponse {
  const context = 'Current user API';
  const record = expectRecord(value, context);
  return {
    user: parseUser(record.user, `${context} user`),
    expiresAtUnixMs: expectNumber(record, 'expiresAtUnixMs', context, {
      integer: true,
      minimum: 0,
    }),
    sessionExpires: expectBoolean(record, 'sessionExpires', context),
  };
}

export function parseCsrfRotation(value: unknown): CsrfRotation {
  const context = 'CSRF API';
  const record = expectRecord(value, context);
  return { csrfToken: expectOpaqueToken(record, 'csrfToken', context) };
}

export function parseHealthResponse(value: unknown): HealthSnapshot {
  const context = 'Health API';
  const record = expectRecord(value, context);

  return {
    status: expectString(record, 'status', context),
    version: expectString(record, 'version', context),
    stateDatabase: expectString(record, 'state_database', context),
    metricsDatabase: expectString(record, 'metrics_database', context),
    timestampUnixMs: expectNumber(record, 'timestamp_unix_ms', context, {
      integer: true,
      minimum: 0,
    }),
  };
}

function parseStorageMount(value: unknown, index: number): StorageMountSnapshot {
  const context = `System overview API storage mount ${index + 1}`;
  const record = expectRecord(value, context);
  const totalBytes = expectU64String(record, 'total_bytes', context);
  const availableBytes = expectU64String(record, 'available_bytes', context);
  const usedBytes = expectU64String(record, 'used_bytes', context);

  if (
    availableBytes > totalBytes ||
    usedBytes > totalBytes ||
    usedBytes + availableBytes !== totalBytes
  ) {
    throw new ApiError(`${context} returned inconsistent capacity values.`);
  }

  return {
    name: expectNullableHostText(record, 'name', context),
    fileSystem: expectNullableHostText(record, 'file_system', context),
    mountPoint: expectNullableHostText(record, 'mount_point', context),
    totalBytes,
    availableBytes,
    usedBytes,
    readOnly: expectBoolean(record, 'read_only', context),
    removable: expectBoolean(record, 'removable', context),
  };
}

function parseStorageOverview(value: unknown): StorageSnapshot {
  const context = 'System overview API storage data';
  const record = expectRecord(value, context);
  const availability = expectDiscoveryAvailability(record, context);
  const mounts = expectArray(
    record,
    'mounts',
    context,
    MAX_STORAGE_MOUNTS,
  ).map(parseStorageMount);
  const omittedMounts = expectNumber(record, 'omitted_mounts', context, {
    integer: true,
    minimum: 0,
  });
  const omittedTextFields = expectNumber(
    record,
    'omitted_text_fields',
    context,
    { integer: true, minimum: 0, maximum: MAX_STORAGE_MOUNTS * 3 },
  );
  const representedNullFields = mounts.reduce(
    (count, mount) =>
      count +
      Number(mount.name === null) +
      Number(mount.fileSystem === null) +
      Number(mount.mountPoint === null),
    0,
  );

  if (representedNullFields !== omittedTextFields) {
    throw new ApiError(`${context} returned an inconsistent omitted_text_fields value.`);
  }
  if (
    availability === 'unavailable' &&
    (mounts.length !== 0 || omittedMounts !== 0 || omittedTextFields !== 0)
  ) {
    throw new ApiError(`${context} returned data for an unavailable collector.`);
  }
  if (
    availability === 'available' &&
    (omittedMounts !== 0 || omittedTextFields !== 0)
  ) {
    throw new ApiError(`${context} marked incomplete data as available.`);
  }
  if (
    availability !== 'degraded' &&
    (omittedMounts !== 0 || omittedTextFields !== 0)
  ) {
    throw new ApiError(`${context} returned omissions without a degraded status.`);
  }

  return { availability, mounts, omittedMounts, omittedTextFields };
}

function addressHasValidShape(address: string, prefixLength: number): boolean {
  if (address.includes(':')) {
    if (prefixLength > 128 || address === ':' || !/^[0-9a-f:.]+$/iu.test(address)) {
      return false;
    }

    try {
      return new URL(`http://[${address}]/`).hostname.length > 2;
    } catch {
      return false;
    }
  }

  if (prefixLength > 32 || !/^\d{1,3}(?:\.\d{1,3}){3}$/u.test(address)) {
    return false;
  }

  return address
    .split('.')
    .every((part) => Number(part) <= 255 && String(Number(part)) === part);
}

function parseNetworkAddress(
  value: unknown,
  interfaceIndex: number,
  addressIndex: number,
): NetworkAddressSnapshot {
  const context =
    `System overview API network interface ${interfaceIndex + 1} ` +
    `address ${addressIndex + 1}`;
  const record = expectRecord(value, context);
  const address = expectHostText(record, 'address', context, 64);
  const prefixLength = expectNumber(record, 'prefix_length', context, {
    integer: true,
    minimum: 0,
    maximum: 128,
  });

  if (!addressHasValidShape(address, prefixLength)) {
    throw new ApiError(`${context} returned an invalid address value.`);
  }

  return { address, prefixLength };
}

function parseNetworkInterface(
  value: unknown,
  index: number,
): NetworkInterfaceSnapshot {
  const context = `System overview API network interface ${index + 1}`;
  const record = expectRecord(value, context);
  const addresses = expectArray(
    record,
    'addresses',
    context,
    MAX_ADDRESSES_PER_INTERFACE,
  ).map((address, addressIndex) => parseNetworkAddress(address, index, addressIndex));

  return {
    name: expectHostText(record, 'name', context),
    addresses,
    totalReceivedBytes: expectU64String(
      record,
      'total_received_bytes',
      context,
    ),
    totalTransmittedBytes: expectU64String(
      record,
      'total_transmitted_bytes',
      context,
    ),
    mtuBytes: expectU64String(record, 'mtu_bytes', context),
  };
}

function parseNetworkOverview(value: unknown): NetworkSnapshot {
  const context = 'System overview API network data';
  const record = expectRecord(value, context);
  const availability = expectDiscoveryAvailability(record, context);
  const interfaces = expectArray(
    record,
    'interfaces',
    context,
    MAX_NETWORK_INTERFACES,
  ).map(parseNetworkInterface);
  const omittedInterfaces = expectNumber(
    record,
    'omitted_interfaces',
    context,
    { integer: true, minimum: 0 },
  );
  const omittedAddresses = expectNumber(
    record,
    'omitted_addresses',
    context,
    { integer: true, minimum: 0 },
  );
  const addressCount = interfaces.reduce(
    (count, networkInterface) => count + networkInterface.addresses.length,
    0,
  );

  if (addressCount > MAX_NETWORK_ADDRESSES) {
    throw new ApiError(`${context} exceeded the total address limit.`);
  }
  if (
    new Set(interfaces.map((networkInterface) => networkInterface.name)).size !==
    interfaces.length
  ) {
    throw new ApiError(`${context} returned duplicate interface names.`);
  }
  if (
    availability === 'unavailable' &&
    (interfaces.length !== 0 || omittedInterfaces !== 0 || omittedAddresses !== 0)
  ) {
    throw new ApiError(`${context} returned data for an unavailable collector.`);
  }
  if (
    availability === 'available' &&
    (omittedInterfaces !== 0 || omittedAddresses !== 0)
  ) {
    throw new ApiError(`${context} marked incomplete data as available.`);
  }
  if (
    availability === 'degraded' &&
    omittedInterfaces === 0 &&
    omittedAddresses === 0
  ) {
    throw new ApiError(`${context} returned a degraded status without omissions.`);
  }

  return { availability, interfaces, omittedInterfaces, omittedAddresses };
}

export function parseSystemOverview(value: unknown): SystemOverview {
  const context = 'System overview API';
  const record = expectRecord(value, context);
  const cpu = expectRecord(record.cpu, `${context} CPU data`);
  const memory = expectRecord(record.memory, `${context} memory data`);
  const swap = expectRecord(record.swap, `${context} swap data`);
  const totalMemoryBytes = expectNumber(
    memory,
    'total_bytes',
    `${context} memory data`,
    { integer: true, minimum: 0 },
  );
  const usedMemoryBytes = expectNumber(
    memory,
    'used_bytes',
    `${context} memory data`,
    { integer: true, minimum: 0 },
  );
  const availableMemoryBytes = expectNumber(
    memory,
    'available_bytes',
    `${context} memory data`,
    { integer: true, minimum: 0 },
  );
  const totalSwapBytes = expectNumber(swap, 'total_bytes', `${context} swap data`, {
    integer: true,
    minimum: 0,
  });
  const usedSwapBytes = expectNumber(swap, 'used_bytes', `${context} swap data`, {
    integer: true,
    minimum: 0,
  });

  if (usedMemoryBytes > totalMemoryBytes || availableMemoryBytes > totalMemoryBytes) {
    throw new ApiError(`${context} returned inconsistent memory values.`);
  }
  if (usedSwapBytes > totalSwapBytes) {
    throw new ApiError(`${context} returned inconsistent swap values.`);
  }

  return {
    hostname: expectNullableHostText(record, 'hostname', context),
    operatingSystem: expectNullableHostText(record, 'operating_system', context),
    architecture: expectHostText(record, 'architecture', context),
    kernelVersion: expectNullableHostText(record, 'kernel_version', context),
    helixVersion: expectHostText(record, 'helix_version', context),
    uptimeSeconds: expectNumber(record, 'uptime_seconds', context, {
      integer: true,
      minimum: 0,
    }),
    cpu: {
      usagePercent: expectNullablePercent(
        cpu,
        'usage_percent',
        `${context} CPU data`,
      ),
      logicalCores: expectNumber(
        cpu,
        'logical_cores',
        `${context} CPU data`,
        { integer: true, minimum: 1 },
      ),
    },
    memory: {
      totalBytes: totalMemoryBytes,
      usedBytes: usedMemoryBytes,
      availableBytes: availableMemoryBytes,
    },
    swap: {
      totalBytes: totalSwapBytes,
      usedBytes: usedSwapBytes,
    },
    storage: parseStorageOverview(record.storage),
    network: parseNetworkOverview(record.network),
    collectedAtUnixMs: expectNumber(record, 'collected_at_unix_ms', context, {
      integer: true,
      minimum: 0,
    }),
  };
}

export interface JsonRequestOptions {
  method?: 'GET' | 'POST' | 'PUT' | 'DELETE';
  body?: unknown;
  csrfToken?: string;
  signal?: AbortSignal | undefined;
  timeoutMs?: number;
}

async function apiProblem(response: Response, path: string): Promise<ApiError> {
  let code: string | null = null;
  let message = `Helix returned HTTP ${response.status} for ${path}.`;
  try {
    const value: unknown = await response.json();
    const record = expectRecord(value, 'Helix API error');
    if (typeof record.code === 'string' && record.code.length > 0) {
      code = record.code;
    }
    if (typeof record.message === 'string' && record.message.length > 0) {
      message = record.message;
    }
  } catch {
    // The HTTP status remains authoritative when an error body is absent or malformed.
  }

  return new ApiError(message, response.status, code);
}

export async function requestJson<T>(
  path: string,
  parser: (value: unknown) => T,
  options: JsonRequestOptions = {},
): Promise<T> {
  const mutation = (options.method ?? 'GET') !== 'GET';
  if (mutation) beginMutationRequest();
  const controller = new AbortController();
  const { signal } = options;
  const cancelRequest = (): void => controller.abort(signal?.reason);
  signal?.addEventListener('abort', cancelRequest, { once: true });
  const timeout = globalThis.setTimeout(
    () => controller.abort(),
    options.timeoutMs ?? REQUEST_TIMEOUT_MS,
  );

  try {
    const response = await fetch(path, {
      method: options.method ?? 'GET',
      headers: {
        Accept: 'application/json',
        ...(options.body === undefined ? {} : { 'Content-Type': 'application/json' }),
        ...(options.csrfToken === undefined
          ? {}
          : { 'X-Helix-CSRF': options.csrfToken }),
      },
      ...(options.body === undefined ? {} : { body: JSON.stringify(options.body) }),
      credentials: 'same-origin',
      cache: 'no-store',
      signal: controller.signal,
    });

    if (!response.ok) {
      throw await apiProblem(response, path);
    }

    let payload: unknown;
    try {
      payload = await response.json();
    } catch {
      throw new ApiError(`${path} did not return valid JSON.`);
    }

    try {
      const dispatchCaptured = mutation
        ? rememberOperationDispatch(path, payload)
        : false;
      const parsed = parser(payload);
      if (mutation && !dispatchCaptured) {
        rememberOperationDispatch(path, parsed);
      }
      return parsed;
    } catch (error) {
      if (error instanceof ApiError) throw error;
      throw new ApiError(`${path} returned data Helix could not safely understand.`);
    }
  } catch (error) {
    if (error instanceof ApiError) {
      throw error;
    }

    if (controller.signal.aborted) {
      if (signal?.aborted === true) {
        throw error;
      }

      throw new ApiError(`The request to ${path} timed out.`);
    }

    throw new ApiError(`Could not reach ${path}.`);
  } finally {
    if (mutation) endMutationRequest();
    globalThis.clearTimeout(timeout);
    signal?.removeEventListener('abort', cancelRequest);
  }
}

export function getHealth(
  csrfToken: string,
  signal?: AbortSignal,
): Promise<HealthSnapshot> {
  return requestJson('/api/v1/health', parseHealthResponse, { csrfToken, signal });
}

export function getSystemOverview(
  csrfToken: string,
  signal?: AbortSignal,
): Promise<SystemOverview> {
  return requestJson('/api/v1/system/overview', parseSystemOverview, {
    csrfToken,
    signal,
  });
}

export function getSetupStatus(signal?: AbortSignal): Promise<SetupStatus> {
  return requestJson('/api/v1/setup/status', parseSetupStatus, { signal });
}

export function setupOwner(input: OwnerSetupInput): Promise<AuthSession> {
  return requestJson('/api/v1/setup/owner', parseAuthSession, {
    method: 'POST',
    body: input,
  });
}

export function login(input: LoginInput): Promise<AuthSession> {
  return requestJson('/api/v1/auth/login', parseAuthSession, {
    method: 'POST',
    body: input,
  });
}

export async function updateAccount(
  input: AccountUpdateInput,
  csrfToken: string,
): Promise<void> {
  beginMutationRequest();
  const controller = new AbortController();
  const timeout = globalThis.setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  try {
    const response = await fetch('/api/v1/auth/account', {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'Content-Type': 'application/json',
        'X-Helix-CSRF': csrfToken,
      },
      body: JSON.stringify(input),
      credentials: 'same-origin',
      cache: 'no-store',
      signal: controller.signal,
    });
    if (!response.ok) throw await apiProblem(response, '/api/v1/auth/account');
    if (response.status !== 204) {
      throw new ApiError('The account update returned an unexpected response.');
    }
  } catch (error) {
    if (error instanceof ApiError) throw error;
    if (controller.signal.aborted) throw new ApiError('The account update timed out.');
    throw new ApiError('Could not reach /api/v1/auth/account.');
  } finally {
    endMutationRequest();
    globalThis.clearTimeout(timeout);
  }
}

export function rotateCsrf(
  csrfToken: string,
  signal?: AbortSignal,
): Promise<CsrfRotation> {
  return requestJson('/api/v1/auth/csrf', parseCsrfRotation, {
    method: 'POST',
    body: {},
    csrfToken,
    signal,
  });
}

export function getCurrentUser(
  csrfToken: string,
  signal?: AbortSignal,
): Promise<AuthenticatedUserResponse> {
  return requestJson('/api/v1/auth/me', parseAuthenticatedUserResponse, {
    csrfToken,
    signal,
  });
}

export async function logout(csrfToken: string): Promise<void> {
  const controller = new AbortController();
  const timeout = globalThis.setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  try {
    const response = await fetch('/api/v1/auth/logout', {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'Content-Type': 'application/json',
        'X-Helix-CSRF': csrfToken,
      },
      body: '{}',
      credentials: 'same-origin',
      cache: 'no-store',
      signal: controller.signal,
    });
    if (!response.ok) {
      throw await apiProblem(response, '/api/v1/auth/logout');
    }
  } catch (error) {
    if (error instanceof ApiError) {
      throw error;
    }
    if (controller.signal.aborted) {
      throw new ApiError('The logout request timed out.');
    }
    throw new ApiError('Could not reach the logout endpoint.');
  } finally {
    globalThis.clearTimeout(timeout);
  }
}
