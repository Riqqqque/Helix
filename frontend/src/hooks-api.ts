import { ApiError, expectArray, expectNumber, expectRecord, expectString, requestJson } from './api';

export type HookServiceAction = 'start' | 'stop' | 'restart' | 'enable' | 'disable';
export type HookKind = 'systemd' | 'api';

export interface HookConnection {
  id: string;
  kind: HookKind;
  unit: string | null;
  installed: boolean;
  active: boolean;
  activeState: string;
  enabled: boolean;
  enabledState: string;
  controllable: boolean;
  actions: HookServiceAction[];
  panelPort: number | null;
  instanceCount: number | null;
  unverifiedInstanceCount: number | null;
  error: string | null;
}

export interface HookInventory {
  hooks: HookConnection[];
  collectedAtUnixMs: number;
}

export interface HookActionResult {
  hookId: string;
  unit: string;
  action: HookServiceAction;
  active: boolean;
  activeState: string;
  enabled: boolean;
  enabledState: string;
  verified: true;
  updatedAtUnixMs: number;
}

const HOOK_ID = /^[a-z][a-z0-9-]{0,63}$/u;
const UNIT = /^[A-Za-z0-9._@-]{1,128}\.service$/u;
const states = ['start', 'stop', 'restart', 'enable', 'disable'] as const;

function bool(record: Record<string, unknown>, key: string, context: string): boolean {
  const value = record[key];
  if (typeof value !== 'boolean') throw new ApiError(`${context} returned an invalid ${key} value.`);
  return value;
}

function text(record: Record<string, unknown>, key: string, context: string): string {
  const value = expectString(record, key, context);
  if (value.length > 256 || Array.from(value).some((character) => /\p{Cc}/u.test(character))) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }
  return value;
}

function nullableText(record: Record<string, unknown>, key: string, context: string): string | null {
  return record[key] === null || record[key] === undefined ? null : text(record, key, context);
}

function nullableCount(record: Record<string, unknown>, key: string, maximum: number): number | null {
  return record[key] === null || record[key] === undefined
    ? null
    : expectNumber(record, key, 'hook connection', { integer: true, minimum: 0, maximum });
}

function parseAction(value: unknown): HookServiceAction {
  if (typeof value !== 'string' || !states.includes(value as HookServiceAction)) {
    throw new ApiError('Hook connection returned an invalid action.');
  }
  return value as HookServiceAction;
}

function parseHook(value: unknown): HookConnection {
  const context = 'hook connection';
  const item = expectRecord(value, context);
  const id = expectString(item, 'id', context);
  const kind = expectString(item, 'kind', context);
  const unit = nullableText(item, 'unit', context);
  if (!HOOK_ID.test(id) || (kind !== 'systemd' && kind !== 'api') || (unit !== null && !UNIT.test(unit))) {
    throw new ApiError('Hook connection returned an invalid identity.');
  }
  const actions = expectArray(item, 'actions', context, 5).map(parseAction);
  if (new Set(actions).size !== actions.length) throw new ApiError('Hook connection returned duplicate actions.');
  const panelPort = nullableCount(item, 'panel_port', 65_535);
  if (panelPort === 0) throw new ApiError('Hook connection returned an invalid panel port.');
  return {
    id,
    kind,
    unit,
    installed: bool(item, 'installed', context),
    active: bool(item, 'active', context),
    activeState: text(item, 'active_state', context),
    enabled: bool(item, 'enabled', context),
    enabledState: text(item, 'enabled_state', context),
    controllable: bool(item, 'controllable', context),
    actions,
    panelPort,
    instanceCount: nullableCount(item, 'instance_count', 100_000),
    unverifiedInstanceCount: nullableCount(item, 'unverified_instance_count', 100_000),
    error: nullableText(item, 'error', context),
  };
}

export function parseHookInventory(value: unknown): HookInventory {
  const root = expectRecord(value, 'hook inventory');
  if (expectNumber(root, 'schema_version', 'hook inventory', { integer: true, minimum: 1, maximum: 1 }) !== 1) {
    throw new ApiError('Hook inventory returned an unsupported schema.');
  }
  const hooks = expectArray(root, 'hooks', 'hook inventory', 33).map(parseHook);
  if (new Set(hooks.map((hook) => hook.id)).size !== hooks.length) {
    throw new ApiError('Hook inventory returned duplicate hooks.');
  }
  return {
    hooks,
    collectedAtUnixMs: expectNumber(root, 'collected_at_unix_ms', 'hook inventory', { integer: true, minimum: 0 }),
  };
}

function parseHookActionResult(value: unknown): HookActionResult {
  const item = expectRecord(value, 'hook action');
  const hookId = expectString(item, 'hook_id', 'hook action');
  const unit = expectString(item, 'unit', 'hook action');
  const action = parseAction(item.action);
  if (!HOOK_ID.test(hookId) || !UNIT.test(unit) || item.verified !== true) {
    throw new ApiError('Hook action did not return verified state.');
  }
  return {
    hookId,
    unit,
    action,
    active: bool(item, 'active', 'hook action'),
    activeState: text(item, 'active_state', 'hook action'),
    enabled: bool(item, 'enabled', 'hook action'),
    enabledState: text(item, 'enabled_state', 'hook action'),
    verified: true,
    updatedAtUnixMs: expectNumber(item, 'updated_at_unix_ms', 'hook action', { integer: true, minimum: 0 }),
  };
}

export function getHookInventory(csrfToken: string, signal?: AbortSignal): Promise<HookInventory> {
  return requestJson('/api/v1/hooks', parseHookInventory, { csrfToken, signal, timeoutMs: 20_000 });
}

export function manageHookService(
  hookId: string,
  action: HookServiceAction,
  csrfToken: string,
): Promise<HookActionResult> {
  if (!HOOK_ID.test(hookId) || !states.includes(action)) {
    return Promise.reject(new ApiError('Choose a valid hook action.'));
  }
  return requestJson(`/api/v1/hooks/${encodeURIComponent(hookId)}/actions`, parseHookActionResult, {
    method: 'POST',
    csrfToken,
    body: { action },
    timeoutMs: 40_000,
  });
}
