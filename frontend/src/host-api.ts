import { expectArray, expectNumber, expectRecord, expectString, requestJson } from './api';

export interface HostServiceIntegration {
  unit: string;
  enabledState: string;
  enabled: boolean;
  activeState: string;
  active: boolean;
}

export interface HostContainerIntegration {
  name: string;
  running: boolean;
  health: string | null;
  restartCount: number;
  restartPolicy: string;
  restartMaximumRetryCount: number;
  oomKilled: boolean;
  stateError: string | null;
}

export interface HelixContainerResources {
  cpuPercent: number | null;
  memoryUsedBytes: number | null;
  pids: number | null;
}

export interface HelixBrokerResources {
  pid: number | null;
  rssBytes: number | null;
  peakRssBytes: number | null;
  threads: number | null;
}

export interface HostIntegrationError {
  component: string;
  code: string;
  message: string;
}

export interface RebootBlocker {
  code: string;
  message: string;
}

export interface RebootActiveServer {
  instanceId: string;
  name: string;
  manager: 'helix_native' | 'amp_import';
  playersOnline: number;
}

export interface RebootActiveJob {
  jobId: string;
  kind: string;
  status: 'queued' | 'running';
  stage: string;
}

export interface HostRebootPreflight {
  canSchedule: boolean;
  activePlayers: number;
  activeServerCount: number;
  activeServers: RebootActiveServer[];
  activeServersTruncated: boolean;
  activeJobsTotal: number;
  activeJobs: RebootActiveJob[];
  blockers: RebootBlocker[];
  checkedAtUnixMs: number;
}

export type ScheduledRebootStatus =
  | { state: 'none'; cancellable: false; staleRecords: number; reconciled: boolean }
  | {
      state: 'scheduled' | 'executing';
      operationId: string;
      scheduledAtUnixMs: number;
      executeAtUnixMs: number;
      delaySeconds: number;
      cancellable: boolean;
    };

export type RebootWeekday =
  | 'monday'
  | 'tuesday'
  | 'wednesday'
  | 'thursday'
  | 'friday'
  | 'saturday'
  | 'sunday';

export type RecurringRebootStatus =
  | { state: 'none'; timerActive: false; timerEnabled: false }
  | { state: 'unavailable'; reason: string }
  | {
      state: 'scheduled' | 'degraded';
      scheduleId: string;
      hostname: string;
      weekdays: RebootWeekday[];
      hour: number;
      minute: number;
      timezone: string;
      calendarExpression: string;
      nextAtUnixMs: number | null;
      timerActive: boolean;
      timerEnabled: boolean;
      missedRunsCatchUp: false;
      executionGate: 'players_jobs_and_inventory_must_be_clear';
      automaticRebootWithoutPreflight: false;
      createdAtUnixMs: number;
      updatedAtUnixMs: number;
    };

export interface HostIntegration {
  availability: 'ready' | 'degraded';
  hostname: string;
  timezone: string | null;
  configuredTargets: { dashboardContainer: string; gatewayContainer: string };
  services: { docker: HostServiceIntegration; helixPrivd: HostServiceIntegration };
  containers: {
    dashboard: HostContainerIntegration | null;
    gateway: HostContainerIntegration | null;
  };
  startOnBoot: {
    state: 'enabled' | 'disabled' | 'mixed' | 'unavailable';
    enabled: boolean | null;
    reconciled: boolean;
    persistence: 'docker_restart_policy';
    currentRuntimeChangedByToggle: boolean;
    containerRecreationMayReset: boolean;
    note: string | null;
  };
  resources: {
    scope: 'helix_only_excludes_game_servers';
    containers: {
      dashboard: HelixContainerResources | null;
      gateway: HelixContainerResources | null;
    };
    broker: HelixBrokerResources;
  };
  scheduledReboot: ScheduledRebootStatus;
  recurringReboot: RecurringRebootStatus;
  rebootPreflight: HostRebootPreflight;
  errors: HostIntegrationError[];
  collectedAtUnixMs: number;
}

export interface StartOnBootResult {
  enabled: boolean;
  policy: 'unless-stopped' | 'no';
  containers: string[];
  changedContainers: number;
  currentRuntimeChanged: boolean;
  persisted: boolean;
  reconciled: boolean;
  containerRecreationMayReset: boolean;
  persistenceNote: string;
  updatedAtUnixMs: number;
}

export interface ScheduledReboot {
  operationId: string;
  state: 'scheduled';
  hostname: string;
  scheduledAtUnixMs: number;
  executeAtUnixMs: number;
  delaySeconds: number;
  cancellable: boolean;
  timerBackend: 'systemd_transient_timer';
  preflight: HostRebootPreflight;
}

export interface CancelledReboot {
  operationId: string;
  state: 'cancelled';
  wasActive: boolean;
  cancelledAtUnixMs: number;
}

export interface RecurringRebootScheduleInput {
  weekdays: RebootWeekday[];
  hour: number;
  minute: number;
  timezone: string;
  confirmationHostname: string;
}

export interface RecurringRebootUpdate {
  state: 'scheduled';
  scheduleId: string;
  hostname: string;
  weekdays: RebootWeekday[];
  hour: number;
  minute: number;
  timezone: string;
  calendarExpression: string;
  nextAtUnixMs: number | null;
  timerActive: true;
  timerEnabled: true;
  missedRunsCatchUp: false;
  executionGate: 'players_jobs_and_inventory_must_be_clear';
  automaticRebootWithoutPreflight: false;
  updatedAtUnixMs: number;
}

export interface RemovedRecurringReboot {
  state: 'removed';
  scheduleId: string;
  hostname: string;
  timerActive: false;
  timerEnabled: false;
  removedAtUnixMs: number;
}

function bool(record: Record<string, unknown>, key: string): boolean {
  const value = record[key];
  if (typeof value !== 'boolean') throw new Error(`Invalid ${key}`);
  return value;
}

function count(record: Record<string, unknown>, key: string): number {
  return expectNumber(record, key, key, { minimum: 0, integer: true });
}

function nullableNumber(record: Record<string, unknown>, key: string): number | null {
  const value = record[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) throw new Error(`Invalid ${key}`);
  return value;
}

function nullableString(record: Record<string, unknown>, key: string): string | null {
  const value = record[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'string' || value.length > 4_096) throw new Error(`Invalid ${key}`);
  return value;
}

function literal<T extends string>(record: Record<string, unknown>, key: string, values: readonly T[]): T {
  const value = expectString(record, key, key);
  if (!values.includes(value as T)) throw new Error(`Invalid ${key}`);
  return value as T;
}

function uuidField(record: Record<string, unknown>, key: string, label: string): string {
  const value = expectString(record, key, label);
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(value)) {
    throw new Error(`Invalid ${label} ID`);
  }
  return value;
}

function operationId(record: Record<string, unknown>): string {
  return uuidField(record, 'operation_id', 'host reboot operation');
}

function parseWeekdays(record: Record<string, unknown>): RebootWeekday[] {
  return expectArray(record, 'weekdays', 'recurring host reboot', 7).map((value) => {
    if (typeof value !== 'string') throw new Error('Invalid recurring reboot weekday');
    const wrapper = { value };
    return literal(wrapper, 'value', [
      'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday',
    ] as const);
  });
}

function parseService(value: unknown): HostServiceIntegration {
  const item = expectRecord(value, 'host service integration');
  return {
    unit: expectString(item, 'unit', 'host service integration'),
    enabledState: expectString(item, 'enabled_state', 'host service integration'),
    enabled: bool(item, 'enabled'),
    activeState: expectString(item, 'active_state', 'host service integration'),
    active: bool(item, 'active'),
  };
}

function parseContainer(value: unknown): HostContainerIntegration | null {
  if (value === null) return null;
  const item = expectRecord(value, 'Helix container integration');
  return {
    name: expectString(item, 'name', 'Helix container integration'),
    running: bool(item, 'running'),
    health: nullableString(item, 'health'),
    restartCount: count(item, 'restart_count'),
    restartPolicy: expectString(item, 'restart_policy', 'Helix container integration'),
    restartMaximumRetryCount: count(item, 'restart_maximum_retry_count'),
    oomKilled: bool(item, 'oom_killed'),
    stateError: nullableString(item, 'state_error'),
  };
}

function parseContainerResources(value: unknown): HelixContainerResources | null {
  if (value === undefined || value === null) return null;
  const item = expectRecord(value, 'Helix container resources');
  return {
    cpuPercent: nullableNumber(item, 'cpu_percent'),
    memoryUsedBytes: nullableNumber(item, 'memory_used_bytes'),
    pids: nullableNumber(item, 'pids'),
  };
}

export function parseHostRebootPreflight(value: unknown): HostRebootPreflight {
  const root = expectRecord(value, 'host reboot preflight');
  if (count(root, 'schema_version') !== 1) throw new Error('Invalid reboot preflight schema');
  return {
    canSchedule: bool(root, 'can_schedule'),
    activePlayers: count(root, 'active_players'),
    activeServerCount: count(root, 'active_server_count'),
    activeServers: expectArray(root, 'active_servers', 'host reboot preflight', 32).map((entry) => {
      const item = expectRecord(entry, 'active server');
      return {
        instanceId: expectString(item, 'instance_id', 'active server'),
        name: expectString(item, 'name', 'active server'),
        manager: literal(item, 'manager', ['helix_native', 'amp_import'] as const),
        playersOnline: count(item, 'players_online'),
      };
    }),
    activeServersTruncated: bool(root, 'active_servers_truncated'),
    activeJobsTotal: count(root, 'active_jobs_total'),
    activeJobs: expectArray(root, 'active_jobs', 'host reboot preflight', 32).map((entry) => {
      const item = expectRecord(entry, 'active host job');
      return {
        jobId: expectString(item, 'job_id', 'active host job'),
        kind: expectString(item, 'kind', 'active host job'),
        status: literal(item, 'status', ['queued', 'running'] as const),
        stage: expectString(item, 'stage', 'active host job'),
      };
    }),
    blockers: expectArray(root, 'blockers', 'host reboot preflight', 32).map((entry) => {
      const item = expectRecord(entry, 'host reboot blocker');
      return {
        code: expectString(item, 'code', 'host reboot blocker'),
        message: expectString(item, 'message', 'host reboot blocker'),
      };
    }),
    checkedAtUnixMs: count(root, 'checked_at_unix_ms'),
  };
}

function parseScheduledReboot(value: unknown): ScheduledRebootStatus {
  const item = expectRecord(value, 'scheduled host reboot');
  const state = literal(item, 'state', ['none', 'scheduled', 'executing'] as const);
  if (state === 'none') {
    return {
      state,
      cancellable: false,
      staleRecords: count(item, 'stale_records'),
      reconciled: bool(item, 'reconciled'),
    };
  }
  return {
    state,
    operationId: operationId(item),
    scheduledAtUnixMs: count(item, 'scheduled_at_unix_ms'),
    executeAtUnixMs: count(item, 'execute_at_unix_ms'),
    delaySeconds: count(item, 'delay_seconds'),
    cancellable: bool(item, 'cancellable'),
  };
}

function parseRecurringReboot(value: unknown): RecurringRebootStatus {
  const item = expectRecord(value, 'recurring host reboot');
  const state = literal(item, 'state', ['none', 'scheduled', 'degraded', 'unavailable'] as const);
  if (state === 'none') {
    if (bool(item, 'timer_active') || bool(item, 'timer_enabled')) throw new Error('Invalid empty recurring reboot state');
    return { state, timerActive: false, timerEnabled: false };
  }
  if (state === 'unavailable') {
    return { state, reason: expectString(item, 'reason', 'recurring host reboot') };
  }
  const missedRunsCatchUp = bool(item, 'missed_runs_catch_up');
  const automaticRebootWithoutPreflight = bool(item, 'automatic_reboot_without_preflight');
  if (missedRunsCatchUp || automaticRebootWithoutPreflight) throw new Error('Invalid recurring reboot safety policy');
  return {
    state,
    scheduleId: uuidField(item, 'schedule_id', 'recurring reboot schedule'),
    hostname: expectString(item, 'hostname', 'recurring host reboot'),
    weekdays: parseWeekdays(item),
    hour: expectNumber(item, 'hour', 'recurring host reboot', { minimum: 0, maximum: 23, integer: true }),
    minute: expectNumber(item, 'minute', 'recurring host reboot', { minimum: 0, maximum: 59, integer: true }),
    timezone: expectString(item, 'timezone', 'recurring host reboot'),
    calendarExpression: expectString(item, 'calendar_expression', 'recurring host reboot'),
    nextAtUnixMs: nullableNumber(item, 'next_at_unix_ms'),
    timerActive: bool(item, 'timer_active'),
    timerEnabled: bool(item, 'timer_enabled'),
    missedRunsCatchUp: false,
    executionGate: literal(item, 'execution_gate', ['players_jobs_and_inventory_must_be_clear'] as const),
    automaticRebootWithoutPreflight: false,
    createdAtUnixMs: count(item, 'created_at_unix_ms'),
    updatedAtUnixMs: count(item, 'updated_at_unix_ms'),
  };
}

export function parseHostIntegration(value: unknown): HostIntegration {
  const root = expectRecord(value, 'host integration');
  if (count(root, 'schema_version') !== 1) throw new Error('Invalid host integration schema');
  const targets = expectRecord(root.configured_targets, 'host integration targets');
  const services = expectRecord(root.services, 'host integration services');
  const containers = expectRecord(root.containers, 'host integration containers');
  const start = expectRecord(root.start_on_boot, 'host start-on-boot');
  const resources = expectRecord(root.resources, 'Helix resources');
  const resourceContainers = expectRecord(resources.containers, 'Helix container resources');
  const broker = expectRecord(resources.broker, 'Helix broker resources');
  return {
    availability: literal(root, 'availability', ['ready', 'degraded'] as const),
    hostname: expectString(root, 'hostname', 'host integration'),
    timezone: nullableString(root, 'timezone'),
    configuredTargets: {
      dashboardContainer: expectString(targets, 'dashboard_container', 'host integration targets'),
      gatewayContainer: expectString(targets, 'gateway_container', 'host integration targets'),
    },
    services: {
      docker: parseService(services.docker),
      helixPrivd: parseService(services.helix_privd),
    },
    containers: {
      dashboard: parseContainer(containers.dashboard),
      gateway: parseContainer(containers.gateway),
    },
    startOnBoot: {
      state: literal(start, 'state', ['enabled', 'disabled', 'mixed', 'unavailable'] as const),
      enabled: start.enabled === null ? null : bool(start, 'enabled'),
      reconciled: bool(start, 'reconciled'),
      persistence: literal(start, 'persistence', ['docker_restart_policy'] as const),
      currentRuntimeChangedByToggle: bool(start, 'current_runtime_changed_by_toggle'),
      containerRecreationMayReset: bool(start, 'container_recreation_may_reset'),
      note: nullableString(start, 'note'),
    },
    resources: {
      scope: literal(resources, 'scope', ['helix_only_excludes_game_servers'] as const),
      containers: {
        dashboard: parseContainerResources(resourceContainers.dashboard),
        gateway: parseContainerResources(resourceContainers.gateway),
      },
      broker: {
        pid: nullableNumber(broker, 'pid'),
        rssBytes: nullableNumber(broker, 'rss_bytes'),
        peakRssBytes: nullableNumber(broker, 'peak_rss_bytes'),
        threads: nullableNumber(broker, 'threads'),
      },
    },
    scheduledReboot: parseScheduledReboot(root.scheduled_reboot),
    recurringReboot: parseRecurringReboot(root.recurring_reboot),
    rebootPreflight: parseHostRebootPreflight(root.reboot_preflight),
    errors: expectArray(root, 'errors', 'host integration', 32).map((entry) => {
      const item = expectRecord(entry, 'host integration error');
      return {
        component: expectString(item, 'component', 'host integration error'),
        code: expectString(item, 'code', 'host integration error'),
        message: expectString(item, 'message', 'host integration error'),
      };
    }),
    collectedAtUnixMs: count(root, 'collected_at_unix_ms'),
  };
}

function parseStartOnBootResult(value: unknown): StartOnBootResult {
  const root = expectRecord(value, 'start-on-boot update');
  return {
    enabled: bool(root, 'enabled'),
    policy: literal(root, 'policy', ['unless-stopped', 'no'] as const),
    containers: expectArray(root, 'containers', 'start-on-boot update', 2).map((value) => {
      if (typeof value !== 'string' || value.trim().length === 0) throw new Error('Invalid start-on-boot container');
      return value;
    }),
    changedContainers: count(root, 'changed_containers'),
    currentRuntimeChanged: bool(root, 'current_runtime_changed'),
    persisted: bool(root, 'persisted'),
    reconciled: bool(root, 'reconciled'),
    containerRecreationMayReset: bool(root, 'container_recreation_may_reset'),
    persistenceNote: expectString(root, 'persistence_note', 'start-on-boot update'),
    updatedAtUnixMs: count(root, 'updated_at_unix_ms'),
  };
}

function parseScheduledRebootResult(value: unknown): ScheduledReboot {
  const root = expectRecord(value, 'scheduled host reboot');
  return {
    operationId: operationId(root),
    state: literal(root, 'state', ['scheduled'] as const),
    hostname: expectString(root, 'hostname', 'scheduled host reboot'),
    scheduledAtUnixMs: count(root, 'scheduled_at_unix_ms'),
    executeAtUnixMs: count(root, 'execute_at_unix_ms'),
    delaySeconds: count(root, 'delay_seconds'),
    cancellable: bool(root, 'cancellable'),
    timerBackend: literal(root, 'timer_backend', ['systemd_transient_timer'] as const),
    preflight: parseHostRebootPreflight(root.preflight),
  };
}

function parseCancelledReboot(value: unknown): CancelledReboot {
  const root = expectRecord(value, 'cancelled host reboot');
  return {
    operationId: operationId(root),
    state: literal(root, 'state', ['cancelled'] as const),
    wasActive: bool(root, 'was_active'),
    cancelledAtUnixMs: count(root, 'cancelled_at_unix_ms'),
  };
}

function parseRecurringRebootUpdate(value: unknown): RecurringRebootUpdate {
  const root = expectRecord(value, 'recurring reboot update');
  const missedRunsCatchUp = bool(root, 'missed_runs_catch_up');
  const automaticRebootWithoutPreflight = bool(root, 'automatic_reboot_without_preflight');
  if (missedRunsCatchUp || automaticRebootWithoutPreflight) throw new Error('Invalid recurring reboot safety policy');
  return {
    state: literal(root, 'state', ['scheduled'] as const),
    scheduleId: uuidField(root, 'schedule_id', 'recurring reboot schedule'),
    hostname: expectString(root, 'hostname', 'recurring reboot update'),
    weekdays: parseWeekdays(root),
    hour: expectNumber(root, 'hour', 'recurring reboot update', { minimum: 0, maximum: 23, integer: true }),
    minute: expectNumber(root, 'minute', 'recurring reboot update', { minimum: 0, maximum: 59, integer: true }),
    timezone: expectString(root, 'timezone', 'recurring reboot update'),
    calendarExpression: expectString(root, 'calendar_expression', 'recurring reboot update'),
    nextAtUnixMs: nullableNumber(root, 'next_at_unix_ms'),
    timerActive: true,
    timerEnabled: true,
    missedRunsCatchUp: false,
    executionGate: literal(root, 'execution_gate', ['players_jobs_and_inventory_must_be_clear'] as const),
    automaticRebootWithoutPreflight: false,
    updatedAtUnixMs: count(root, 'updated_at_unix_ms'),
  };
}

function parseRemovedRecurringReboot(value: unknown): RemovedRecurringReboot {
  const root = expectRecord(value, 'removed recurring reboot');
  if (bool(root, 'timer_active') || bool(root, 'timer_enabled')) throw new Error('Invalid removed recurring reboot state');
  return {
    state: literal(root, 'state', ['removed'] as const),
    scheduleId: uuidField(root, 'schedule_id', 'recurring reboot schedule'),
    hostname: expectString(root, 'hostname', 'removed recurring reboot'),
    timerActive: false,
    timerEnabled: false,
    removedAtUnixMs: count(root, 'removed_at_unix_ms'),
  };
}

export function getHostIntegration(csrfToken: string, signal?: AbortSignal): Promise<HostIntegration> {
  return requestJson('/api/v1/host/integration', parseHostIntegration, { csrfToken, signal, timeoutMs: 25_000 });
}

export function setHelixStartOnBoot(enabled: boolean, csrfToken: string): Promise<StartOnBootResult> {
  return requestJson('/api/v1/host/integration/start-on-boot', parseStartOnBootResult, {
    method: 'PUT', body: { enabled }, csrfToken, timeoutMs: 25_000,
  });
}

export function getHostRebootPreflight(csrfToken: string): Promise<HostRebootPreflight> {
  return requestJson('/api/v1/host/reboot/preflight', parseHostRebootPreflight, { csrfToken, timeoutMs: 25_000 });
}

export function scheduleHostReboot(hostname: string, delaySeconds: number, csrfToken: string): Promise<ScheduledReboot> {
  return requestJson('/api/v1/host/reboot', parseScheduledRebootResult, {
    method: 'POST',
    body: { confirmation_hostname: hostname, delay_seconds: delaySeconds, disruption_acknowledged: true },
    csrfToken,
    timeoutMs: 25_000,
  });
}

export function cancelHostReboot(id: string, csrfToken: string): Promise<CancelledReboot> {
  return requestJson(`/api/v1/host/reboot/${encodeURIComponent(id)}`, parseCancelledReboot, {
    method: 'DELETE', body: {}, csrfToken, timeoutMs: 25_000,
  });
}

export function setRecurringHostReboot(
  schedule: RecurringRebootScheduleInput,
  csrfToken: string,
): Promise<RecurringRebootUpdate> {
  return requestJson('/api/v1/host/reboot/recurring', parseRecurringRebootUpdate, {
    method: 'PUT',
    body: {
      weekdays: schedule.weekdays,
      hour: schedule.hour,
      minute: schedule.minute,
      timezone: schedule.timezone,
      confirmation_hostname: schedule.confirmationHostname,
      disruption_acknowledged: true,
    },
    csrfToken,
    timeoutMs: 30_000,
  });
}

export function deleteRecurringHostReboot(hostname: string, csrfToken: string): Promise<RemovedRecurringReboot> {
  return requestJson('/api/v1/host/reboot/recurring', parseRemovedRecurringReboot, {
    method: 'DELETE', body: { confirmation_hostname: hostname }, csrfToken, timeoutMs: 30_000,
  });
}
