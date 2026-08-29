import { ApiError, expectArray, expectNumber, expectRecord, expectString, requestJson } from './api';

export interface SecurityControl {
  id: string;
  title: string;
  summary: string;
  offReason: string;
  state: string;
  enabled: boolean;
  writable: boolean;
  recommended: boolean;
  implications: string;
  confirmationEnable: string | null;
  confirmationDisable: string | null;
}

export interface SecurityTip {
  id: string;
  title: string;
  body: string;
}

export interface SecurityInventory {
  controls: SecurityControl[];
  tips: SecurityTip[];
  facts: Record<string, string | null>;
  collectedAtUnixMs: number;
}

const CONTROL_ID = /^[a-z][a-z0-9_]{1,63}$/u;

function bool(record: Record<string, unknown>, key: string, context: string): boolean {
  const value = record[key];
  if (typeof value !== 'boolean') throw new ApiError(`${context} returned an invalid ${key} value.`);
  return value;
}

function text(record: Record<string, unknown>, key: string, context: string, maximum = 2_048): string {
  const value = expectString(record, key, context);
  if (value.length > maximum || Array.from(value).some((character) => /\p{Cc}/u.test(character))) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }
  return value;
}

function optionalText(record: Record<string, unknown>, key: string, maximum = 128): string | null {
  const value = record[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'string' || value.length > maximum || Array.from(value).some((character) => /\p{Cc}/u.test(character))) {
    throw new ApiError(`Security inventory returned an invalid ${key} value.`);
  }
  return value;
}

function parseControl(value: unknown): SecurityControl {
  const item = expectRecord(value, 'security control');
  const id = expectString(item, 'id', 'security control');
  if (!CONTROL_ID.test(id)) throw new ApiError('Security inventory returned an invalid control.');
  return {
    id,
    title: text(item, 'title', 'security control', 120),
    summary: text(item, 'summary', 'security control', 800),
    offReason: text(item, 'off_reason', 'security control', 800),
    state: text(item, 'state', 'security control', 64),
    enabled: bool(item, 'enabled', 'security control'),
    writable: bool(item, 'writable', 'security control'),
    recommended: bool(item, 'recommended', 'security control'),
    implications: text(item, 'implications', 'security control', 1_200),
    confirmationEnable: optionalText(item, 'confirmation_enable'),
    confirmationDisable: optionalText(item, 'confirmation_disable'),
  };
}

function parseTip(value: unknown): SecurityTip {
  const item = expectRecord(value, 'security tip');
  const id = expectString(item, 'id', 'security tip');
  if (!CONTROL_ID.test(id)) throw new ApiError('Security inventory returned an invalid tip.');
  return {
    id,
    title: text(item, 'title', 'security tip', 120),
    body: text(item, 'body', 'security tip', 800),
  };
}

export function parseSecurityInventory(value: unknown): SecurityInventory {
  const root = expectRecord(value, 'security inventory');
  if (expectNumber(root, 'schema_version', 'security inventory', { integer: true, minimum: 1, maximum: 1 }) !== 1) {
    throw new ApiError('Security inventory returned an unsupported schema.');
  }
  const factsRecord = expectRecord(root.facts, 'security facts');
  const facts: Record<string, string | null> = {};
  for (const [key, item] of Object.entries(factsRecord)) {
    if (!CONTROL_ID.test(key.replaceAll('-', '_')) && !/^[a-z][a-z0-9_]{0,63}$/u.test(key)) {
      throw new ApiError('Security inventory returned an invalid fact.');
    }
    if (item === null) facts[key] = null;
    else if (typeof item === 'string' && item.length <= 240 && !Array.from(item).some((character) => /\p{Cc}/u.test(character))) {
      facts[key] = item;
    } else if (typeof item === 'boolean' || typeof item === 'number') {
      facts[key] = String(item);
    } else {
      throw new ApiError('Security inventory returned an invalid fact value.');
    }
  }
  return {
    controls: expectArray(root, 'controls', 'security inventory', 64).map(parseControl),
    tips: root.tips === undefined ? [] : expectArray(root, 'tips', 'security inventory', 24).map(parseTip),
    facts,
    collectedAtUnixMs: expectNumber(root, 'collected_at_unix_ms', 'security inventory', { integer: true, minimum: 0 }),
  };
}

function followAbort<T>(promise: Promise<T>, signal?: AbortSignal): Promise<T> {
  if (signal === undefined) return promise;
  return new Promise((resolve, reject) => {
    const abort = (): void => {
      reject(signal.reason ?? new DOMException('Aborted', 'AbortError'));
    };
    if (signal.aborted) {
      abort();
      return;
    }
    signal.addEventListener('abort', abort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener('abort', abort);
        resolve(value);
      },
      (error: unknown) => {
        signal.removeEventListener('abort', abort);
        reject(error);
      },
    );
  });
}

const SECURITY_INVENTORY_CACHE_MS = 8_000;
let securityInventoryInflight: { csrf: string; promise: Promise<SecurityInventory> } | null = null;
let securityInventoryCache: { csrf: string; at: number; value: SecurityInventory } | null = null;

export function resetSecurityInventoryPrefetch(): void {
  securityInventoryInflight = null;
  securityInventoryCache = null;
}

export function prefetchSecurityInventory(csrfToken: string): void {
  void getSecurityInventory(csrfToken).catch(() => undefined);
}

export function getSecurityInventory(
  csrfToken: string,
  signal?: AbortSignal,
  options?: { fresh?: boolean },
): Promise<SecurityInventory> {
  const fresh = options?.fresh === true;
  if (
    !fresh
    && securityInventoryCache !== null
    && securityInventoryCache.csrf === csrfToken
    && Date.now() - securityInventoryCache.at < SECURITY_INVENTORY_CACHE_MS
  ) {
    return followAbort(Promise.resolve(securityInventoryCache.value), signal);
  }
  if (!fresh && securityInventoryInflight !== null && securityInventoryInflight.csrf === csrfToken) {
    return followAbort(securityInventoryInflight.promise, signal);
  }
  const promise = requestJson('/api/v1/security', parseSecurityInventory, { csrfToken, timeoutMs: 20_000 }).then((value) => {
    securityInventoryCache = { csrf: csrfToken, at: Date.now(), value };
    return value;
  });
  securityInventoryInflight = { csrf: csrfToken, promise };
  void promise.finally(() => {
    if (securityInventoryInflight?.promise === promise) securityInventoryInflight = null;
  });
  return followAbort(promise, signal);
}

export function setSecurityControl(
  id: string,
  enabled: boolean,
  confirmation: string,
  csrfToken: string,
): Promise<Record<string, unknown>> {
  if (!CONTROL_ID.test(id) || confirmation.trim().length === 0) {
    return Promise.reject(new ApiError('That security change is not valid.'));
  }
  return requestJson('/api/v1/security/controls', (value) => expectRecord(value, 'security control result'), {
    method: 'POST',
    csrfToken,
    body: { id, enabled, confirmation },
  });
}
