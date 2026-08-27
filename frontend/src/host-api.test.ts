import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  cancelHostReboot,
  deleteRecurringHostReboot,
  getHostIntegration,
  getHostRebootPreflight,
  parseHostIntegration,
  scheduleHostReboot,
  setRecurringHostReboot,
  setHelixStartOnBoot,
} from './host-api';

const operationId = '6b8f95ce-9c58-4c4c-b232-627a29ca1c03';
const preflight = {
  schema_version: 1,
  can_schedule: true,
  active_players: 0,
  active_server_count: 0,
  active_servers: [],
  active_servers_truncated: false,
  active_jobs_total: 0,
  active_jobs: [],
  blockers: [],
  checked_at_unix_ms: 1_800_000_000_000,
};
const integration = {
  schema_version: 1,
  availability: 'ready',
  hostname: 'helix-host',
  timezone: 'America/Denver',
  configured_targets: { dashboard_container: 'helix', gateway_container: 'helix-gateway' },
  services: {
    docker: { unit: 'docker.service', enabled_state: 'enabled', enabled: true, active_state: 'active', active: true },
    helix_privd: { unit: 'helix-privd.service', enabled_state: 'enabled', enabled: true, active_state: 'active', active: true },
  },
  containers: {
    dashboard: { name: 'helix', running: true, health: 'healthy', restart_count: 0, restart_policy: 'unless-stopped', restart_maximum_retry_count: 0, oom_killed: false, state_error: null },
    gateway: { name: 'helix-gateway', running: true, health: null, restart_count: 1, restart_policy: 'unless-stopped', restart_maximum_retry_count: 0, oom_killed: false, state_error: null },
  },
  start_on_boot: { state: 'enabled', enabled: true, reconciled: true, persistence: 'docker_restart_policy', current_runtime_changed_by_toggle: false, container_recreation_may_reset: true, note: 'Applied to both containers.' },
  resources: {
    scope: 'helix_only_excludes_game_servers',
    containers: { dashboard: { cpu_percent: 0.7, memory_used_bytes: 86_000_000, pids: 9 }, gateway: { cpu_percent: 0.1, memory_used_bytes: 14_000_000, pids: 2 } },
    broker: { pid: 44, rss_bytes: 9_000_000, peak_rss_bytes: 12_000_000, threads: 3 },
  },
  scheduled_reboot: { state: 'none', cancellable: false, stale_records: 0, reconciled: true },
  recurring_reboot: { state: 'none', timer_active: false, timer_enabled: false },
  reboot_preflight: preflight,
  errors: [],
  collected_at_unix_ms: 1_800_000_000_100,
};

afterEach(() => vi.unstubAllGlobals());

describe('host integration API', () => {
  it('parses services, containers, Helix-only resources, and reboot state', () => {
    expect(parseHostIntegration(integration)).toMatchObject({
      hostname: 'helix-host',
      startOnBoot: { state: 'enabled', enabled: true },
      resources: { scope: 'helix_only_excludes_game_servers', broker: { rssBytes: 9_000_000 } },
      scheduledReboot: { state: 'none' },
      recurringReboot: { state: 'none' },
      rebootPreflight: { canSchedule: true },
    });
  });

  it('accepts unavailable optional runtime data without inventing values', () => {
    const parsed = parseHostIntegration({
      ...integration,
      availability: 'degraded',
      containers: { dashboard: null, gateway: null },
      start_on_boot: { ...integration.start_on_boot, state: 'unavailable', enabled: null, note: undefined },
      resources: { scope: 'helix_only_excludes_game_servers', containers: {}, broker: {} },
      errors: [{ component: 'docker', code: 'unavailable', message: 'Docker could not be queried.' }],
    });
    expect(parsed.startOnBoot.enabled).toBeNull();
    expect(parsed.resources.containers.dashboard).toBeNull();
    expect(parsed.resources.broker.rssBytes).toBeNull();
    expect(parsed.errors).toHaveLength(1);
  });

  it('rejects unsupported schemas and impossible states', () => {
    expect(() => parseHostIntegration({ ...integration, schema_version: 2 })).toThrow();
    expect(() => parseHostIntegration({ ...integration, availability: 'unknown' })).toThrow();
  });

  it('uses the exact host integration, preflight, schedule, and cancel contracts', async () => {
    const startResult = {
      enabled: true,
      policy: 'unless-stopped',
      containers: ['helix', 'helix-gateway'],
      changed_containers: 2,
      current_runtime_changed: false,
      persisted: true,
      reconciled: true,
      container_recreation_may_reset: true,
      persistence_note: 'Container recreation may reapply Compose policy.',
      updated_at_unix_ms: 1_800_000_000_200,
    };
    const scheduled = {
      operation_id: operationId,
      state: 'scheduled',
      hostname: 'helix-host',
      scheduled_at_unix_ms: 1_800_000_000_300,
      execute_at_unix_ms: 1_800_000_030_300,
      delay_seconds: 30,
      cancellable: true,
      timer_backend: 'systemd_transient_timer',
      preflight,
    };
    const cancelled = { operation_id: operationId, state: 'cancelled', was_active: true, cancelled_at_unix_ms: 1_800_000_001_000 };
    const recurring = {
      state: 'scheduled',
      schedule_id: operationId,
      hostname: 'helix-host',
      weekdays: ['monday', 'wednesday', 'friday'],
      hour: 5,
      minute: 30,
      timezone: 'America/Denver',
      calendar_expression: 'Mon,Wed,Fri *-*-* 05:30:00 America/Denver',
      next_at_unix_ms: 1_800_050_000_000,
      timer_active: true,
      timer_enabled: true,
      missed_runs_catch_up: false,
      execution_gate: 'players_jobs_and_inventory_must_be_clear',
      automatic_reboot_without_preflight: false,
      updated_at_unix_ms: 1_800_000_001_100,
    };
    const recurringRemoved = {
      state: 'removed', schedule_id: operationId, hostname: 'helix-host', timer_active: false,
      timer_enabled: false, removed_at_unix_ms: 1_800_000_001_200,
    };
    const responses = [integration, startResult, preflight, scheduled, cancelled, recurring, recurringRemoved];
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(new Response(JSON.stringify(responses.shift()), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    })));
    vi.stubGlobal('fetch', fetchMock);
    const csrf = 'EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE';

    await getHostIntegration(csrf);
    await setHelixStartOnBoot(true, csrf);
    await getHostRebootPreflight(csrf);
    await scheduleHostReboot('helix-host', 30, csrf);
    await cancelHostReboot(operationId, csrf);
    await setRecurringHostReboot({
      weekdays: ['monday', 'wednesday', 'friday'], hour: 5, minute: 30,
      timezone: 'America/Denver', confirmationHostname: 'helix-host',
    }, csrf);
    await deleteRecurringHostReboot('helix-host', csrf);

    expect(fetchMock.mock.calls.map((call) => call[0])).toEqual([
      '/api/v1/host/integration',
      '/api/v1/host/integration/start-on-boot',
      '/api/v1/host/reboot/preflight',
      '/api/v1/host/reboot',
      `/api/v1/host/reboot/${operationId}`,
      '/api/v1/host/reboot/recurring',
      '/api/v1/host/reboot/recurring',
    ]);
    const put = fetchMock.mock.calls[1]?.[1] as RequestInit;
    expect(put.method).toBe('PUT');
    expect(JSON.parse(String(put.body))).toEqual({ enabled: true });
    const post = fetchMock.mock.calls[3]?.[1] as RequestInit;
    expect(post.method).toBe('POST');
    expect(JSON.parse(String(post.body))).toEqual({ confirmation_hostname: 'helix-host', delay_seconds: 30, disruption_acknowledged: true });
    const remove = fetchMock.mock.calls[4]?.[1] as RequestInit;
    expect(remove.method).toBe('DELETE');
    expect(remove.headers).toMatchObject({ 'Content-Type': 'application/json', 'X-Helix-CSRF': csrf });
    expect(remove.body).toBe('{}');
    const recurringPut = fetchMock.mock.calls[5]?.[1] as RequestInit;
    expect(recurringPut.method).toBe('PUT');
    expect(JSON.parse(String(recurringPut.body))).toEqual({
      weekdays: ['monday', 'wednesday', 'friday'], hour: 5, minute: 30,
      timezone: 'America/Denver', confirmation_hostname: 'helix-host', disruption_acknowledged: true,
    });
    const recurringDelete = fetchMock.mock.calls[6]?.[1] as RequestInit;
    expect(recurringDelete.method).toBe('DELETE');
    expect(JSON.parse(String(recurringDelete.body))).toEqual({ confirmation_hostname: 'helix-host' });
  });
});
