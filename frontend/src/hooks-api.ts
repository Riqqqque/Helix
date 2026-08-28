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

export type HookInstallMode = 'one_click' | 'guided';
export type HookInstallStatus = 'ready' | 'needs_input' | 'blocked';

export interface HookInstallPlan {
  hookId: string;
  mode: HookInstallMode;
  installAvailable: boolean;
  status: HookInstallStatus;
  platform: { id: string; name: string; codename: string; architecture: string } | null;
  checks: Array<{ id: string; label: string; status: 'pass' | 'warning' | 'block'; detail: string }>;
  changes: string[];
  nextSteps: string[];
  blockers: string[];
  officialDocs: string;
  collectedAtUnixMs: number;
}

export interface HookInstallJob {
  id: string;
  kind: 'hook_install';
  status: 'queued' | 'running' | 'complete' | 'failed';
  stage: string;
  progressPercent: number;
  result: Record<string, unknown> | null;
  error: string | null;
}

const HOOK_ID = /^[a-z][a-z0-9-]{0,63}$/u;
const UNIT = /^[A-Za-z0-9._@-]{1,128}\.service$/u;
const JOB_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;
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

function boundedStringList(record: Record<string, unknown>, key: string, context: string, maximum: number): string[] {
  return expectArray(record, key, context, maximum).map((value) => {
    if (typeof value !== 'string' || value.length === 0 || value.length > 1_024) {
      throw new ApiError(`${context} returned an invalid ${key} value.`);
    }
    return value;
  });
}

export function parseHookInstallPlan(value: unknown): HookInstallPlan {
  const root = expectRecord(value, 'hook install plan');
  if (expectNumber(root, 'schema_version', 'hook install plan', { integer: true, minimum: 1, maximum: 1 }) !== 1) {
    throw new ApiError('Hook install plan returned an unsupported schema.');
  }
  const hookId = expectString(root, 'hook_id', 'hook install plan');
  const mode = expectString(root, 'mode', 'hook install plan');
  const status = expectString(root, 'status', 'hook install plan');
  if (!HOOK_ID.test(hookId) || (mode !== 'one_click' && mode !== 'guided') || !['ready', 'needs_input', 'blocked'].includes(status)) {
    throw new ApiError('Hook install plan returned an invalid classification.');
  }
  const platformRoot = root.platform === null ? null : expectRecord(root.platform, 'hook install platform');
  const checks = expectArray(root, 'checks', 'hook install plan', 16).map((value) => {
    const item = expectRecord(value, 'hook install check');
    const checkStatus = expectString(item, 'status', 'hook install check');
    if (!['pass', 'warning', 'block'].includes(checkStatus)) throw new ApiError('Hook install plan returned an invalid check status.');
    return {
      id: expectString(item, 'id', 'hook install check'),
      label: expectString(item, 'label', 'hook install check'),
      status: checkStatus as 'pass' | 'warning' | 'block',
      detail: expectString(item, 'detail', 'hook install check'),
    };
  });
  const installAvailable = bool(root, 'install_available', 'hook install plan');
  const blockers = boundedStringList(root, 'blockers', 'hook install plan', 16);
  if (installAvailable !== (mode === 'one_click' && status === 'ready' && blockers.length === 0)) {
    throw new ApiError('Hook install plan returned inconsistent availability.');
  }
  const officialDocs = expectString(root, 'official_docs', 'hook install plan');
  if (!officialDocs.startsWith('https://') || officialDocs.length > 2_048) {
    throw new ApiError('Hook install plan returned invalid official documentation.');
  }
  return {
    hookId,
    mode: mode as HookInstallMode,
    installAvailable,
    status: status as HookInstallStatus,
    platform: platformRoot === null ? null : {
      id: text(platformRoot, 'id', 'hook install platform'),
      name: text(platformRoot, 'name', 'hook install platform'),
      codename: text(platformRoot, 'codename', 'hook install platform'),
      architecture: text(platformRoot, 'architecture', 'hook install platform'),
    },
    checks,
    changes: boundedStringList(root, 'changes', 'hook install plan', 16),
    nextSteps: boundedStringList(root, 'next_steps', 'hook install plan', 16),
    blockers,
    officialDocs,
    collectedAtUnixMs: expectNumber(root, 'collected_at_unix_ms', 'hook install plan', { integer: true, minimum: 0 }),
  };
}

export function parseHookInstallJob(value: unknown): HookInstallJob {
  const root = expectRecord(value, 'hook install job');
  const kind = expectString(root, 'kind', 'hook install job');
  const status = expectString(root, 'status', 'hook install job');
  if (kind !== 'hook_install' || !['queued', 'running', 'complete', 'failed'].includes(status)) {
    throw new ApiError('Hook install job returned an invalid state.');
  }
  const result = root.result === null || root.result === undefined ? null : expectRecord(root.result, 'hook install result');
  const error = root.error === null || root.error === undefined ? null : expectString(root, 'error', 'hook install job');
  const id = expectString(root, 'id', 'hook install job');
  if (!JOB_ID.test(id)) throw new ApiError('Hook install job returned an invalid ID.');
  return {
    id,
    kind,
    status: status as HookInstallJob['status'],
    stage: expectString(root, 'stage', 'hook install job'),
    progressPercent: expectNumber(root, 'progress_percent', 'hook install job', { integer: true, minimum: 0, maximum: 100 }),
    result,
    error,
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

export function getHookInstallPlan(hookId: string, csrfToken: string, signal?: AbortSignal): Promise<HookInstallPlan> {
  if (!HOOK_ID.test(hookId)) return Promise.reject(new ApiError('Choose a valid hook.'));
  return requestJson(`/api/v1/hooks/${encodeURIComponent(hookId)}/install/preflight`, parseHookInstallPlan, {
    csrfToken,
    signal,
    timeoutMs: 20_000,
  });
}

export function installHook(hookId: string, csrfToken: string): Promise<{ jobId: string; reused: boolean }> {
  if (!HOOK_ID.test(hookId)) return Promise.reject(new ApiError('Choose a valid hook.'));
  return requestJson(`/api/v1/hooks/${encodeURIComponent(hookId)}/install`, (value) => {
    const root = expectRecord(value, 'hook install dispatch');
    const jobId = expectString(root, 'job_id', 'hook install dispatch');
    if (!JOB_ID.test(jobId)) throw new ApiError('Hook install dispatch returned an invalid job ID.');
    return {
      jobId,
      reused: bool(root, 'reused', 'hook install dispatch'),
    };
  }, {
    method: 'POST',
    body: { confirmation: hookId, repository_change_acknowledged: true },
    csrfToken,
    timeoutMs: 20_000,
  });
}

export function getHookInstallJob(jobId: string, csrfToken: string, signal?: AbortSignal): Promise<HookInstallJob> {
  if (!JOB_ID.test(jobId)) return Promise.reject(new ApiError('Hook install job ID is invalid.'));
  return requestJson(`/api/v1/hooks/jobs/${encodeURIComponent(jobId)}`, parseHookInstallJob, {
    csrfToken,
    signal,
  });
}
