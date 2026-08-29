import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  getHookInstallJob,
  getHookInstallPlan,
  getHookInventory,
  installHook,
  manageHookService,
  parseHookInstallPlan,
  parseHookInventory,
  resetHookInventoryPrefetch,
} from './hooks-api';

const inventory = {
  schema_version: 1,
  hooks: [
    { id: 'plex', kind: 'systemd', unit: 'plexmediaserver.service', installed: true, active: true, active_state: 'active', enabled: true, enabled_state: 'enabled', controllable: true, actions: ['start', 'stop', 'restart', 'enable', 'disable'], panel_port: null, instance_count: null, unverified_instance_count: null, memory_used_bytes: 128_000_000, cpu_percent: null, error: null },
    { id: 'amp', kind: 'api', installed: true, active: true, active_state: 'connected', enabled: true, enabled_state: 'configured', controllable: false, actions: [], panel_port: 8080, instance_count: 4, unverified_instance_count: 0, memory_used_bytes: null, cpu_percent: null, error: null },
    { id: 'docker', kind: 'docker', installed: true, active: true, active_state: 'running', enabled: true, enabled_state: 'installed', controllable: false, actions: [], panel_port: 9000, instance_count: 8, unverified_instance_count: null, memory_used_bytes: 2_147_483_648, cpu_percent: 12.5, error: null },
  ],
  collected_at_unix_ms: 1_800_000_000_000,
};

const installPlan = {
  schema_version: 1,
  hook_id: 'tailscale',
  mode: 'one_click',
  install_available: true,
  status: 'ready',
  platform: { id: 'ubuntu', name: 'Ubuntu 24.04.3 LTS', codename: 'noble', architecture: 'amd64' },
  checks: [{ id: 'apt', label: 'APT package manager', status: 'pass', detail: 'Available' }],
  changes: ['Add the official signed repository', 'Install tailscale'],
  next_steps: ['Approve this machine in the intended tailnet'],
  blockers: [],
  official_docs: 'https://tailscale.com/docs/install/linux',
  collected_at_unix_ms: 1_800_000_000_000,
};

afterEach(() => {
  vi.unstubAllGlobals();
  resetHookInventoryPrefetch();
});

describe('hooks API', () => {
  it('keeps systemd control and external API connections distinct', () => {
    const parsed = parseHookInventory(inventory);
    expect(parsed.hooks[0]).toMatchObject({ id: 'plex', kind: 'systemd', controllable: true, memoryUsedBytes: 128_000_000 });
    expect(parsed.hooks[1]).toMatchObject({ id: 'amp', kind: 'api', panelPort: 8080, instanceCount: 4 });
    expect(parsed.hooks[2]).toMatchObject({ id: 'docker', kind: 'docker', panelPort: 9000, cpuPercent: 12.5 });
  });

  it('rejects duplicate identities and invented actions', () => {
    expect(() => parseHookInventory({ ...inventory, hooks: [inventory.hooks[0], inventory.hooks[0]] })).toThrow();
    expect(() => parseHookInventory({ ...inventory, hooks: [{ ...inventory.hooks[0], actions: ['reboot-host'] }] })).toThrow();
  });

  it('reuses an in-flight inventory request for prefetch and page load', async () => {
    let resolveFetch: ((value: Response) => void) | undefined;
    const fetchMock = vi.fn().mockImplementation(
      () => new Promise<Response>((resolve) => {
        resolveFetch = resolve;
      }),
    );
    vi.stubGlobal('fetch', fetchMock);
    const csrf = 'EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE';

    const first = getHookInventory(csrf);
    const second = getHookInventory(csrf);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    resolveFetch?.(new Response(JSON.stringify(inventory), { status: 200, headers: { 'Content-Type': 'application/json' } }));
    const [firstInventory, secondInventory] = await Promise.all([first, second]);
    expect(firstInventory.hooks.map((hook) => hook.id)).toEqual(['plex', 'amp', 'docker']);
    expect(secondInventory).toBe(firstInventory);
  });

  it('uses exact inventory and typed action routes', async () => {
    const action = { hook_id: 'plex', unit: 'plexmediaserver.service', action: 'restart', active: true, active_state: 'active', enabled: true, enabled_state: 'enabled', verified: true, updated_at_unix_ms: 1_800_000_000_100 };
    const responses = [inventory, action];
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(new Response(JSON.stringify(responses.shift()), { status: 200, headers: { 'Content-Type': 'application/json' } })));
    vi.stubGlobal('fetch', fetchMock);
    const csrf = 'EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE';

    await getHookInventory(csrf);
    await manageHookService('plex', 'restart', csrf);

    expect(fetchMock.mock.calls.map((call) => call[0])).toEqual(['/api/v1/hooks', '/api/v1/hooks/plex/actions']);
    expect(JSON.parse(String((fetchMock.mock.calls[1]?.[1] as RequestInit).body))).toEqual({ action: 'restart' });
  });

  it('requires one-click readiness, no blockers, and official HTTPS documentation', () => {
    expect(parseHookInstallPlan(installPlan)).toMatchObject({ hookId: 'tailscale', installAvailable: true, status: 'ready', writes: [] });
    expect(() => parseHookInstallPlan({ ...installPlan, blockers: ['APT is unavailable'] })).toThrow(/inconsistent/i);
    expect(() => parseHookInstallPlan({ ...installPlan, official_docs: 'javascript:alert(1)' })).toThrow(/documentation/i);
  });

  it('accepts exact write paths and rejects traversal', () => {
    const parsed = parseHookInstallPlan({
      ...installPlan,
      writes: [
        { path: '/run/helix/hook-installs', kind: 'staging' },
        { path: '/usr/share/keyrings/tailscale-archive-keyring.gpg', kind: 'keyring' },
        { path: '/etc/apt/sources.list.d/tailscale.list', kind: 'source' },
      ],
    });
    expect(parsed.writes).toEqual([
      { path: '/run/helix/hook-installs', kind: 'staging' },
      { path: '/usr/share/keyrings/tailscale-archive-keyring.gpg', kind: 'keyring' },
      { path: '/etc/apt/sources.list.d/tailscale.list', kind: 'source' },
    ]);
    expect(() => parseHookInstallPlan({
      ...installPlan,
      writes: [{ path: '/etc/apt/../passwd', kind: 'source' }],
    })).toThrow(/write path/i);
    expect(() => parseHookInstallPlan({
      ...installPlan,
      writes: [{ path: 'etc/apt/sources.list.d/tailscale.list', kind: 'source' }],
    })).toThrow(/write path/i);
  });

  it('uses exact preflight, install, and opaque job routes', async () => {
    const jobId = '8953dc16-3891-42bf-802f-711b3ba2965a';
    const responses = [
      installPlan,
      { job_id: jobId, reused: false },
      { id: jobId, kind: 'hook_install', status: 'running', stage: 'Installing', progress_percent: 52, result: null, error: null },
    ];
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(new Response(JSON.stringify(responses.shift()), { status: 200, headers: { 'Content-Type': 'application/json' } })));
    vi.stubGlobal('fetch', fetchMock);

    await getHookInstallPlan('tailscale', 'csrf');
    await installHook('tailscale', 'csrf');
    await getHookInstallJob(jobId, 'csrf');

    expect(fetchMock.mock.calls.map((call) => call[0])).toEqual([
      '/api/v1/hooks/tailscale/install/preflight',
      '/api/v1/hooks/tailscale/install',
      `/api/v1/hooks/jobs/${jobId}`,
    ]);
    expect(JSON.parse(String((fetchMock.mock.calls[1]?.[1] as RequestInit).body))).toEqual({
      confirmation: 'tailscale',
      repository_change_acknowledged: true,
    });
  });

  it('rejects non-opaque hook and job identities before fetch', async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    await expect(getHookInstallPlan('../tailscale', 'csrf')).rejects.toThrow(/valid hook/i);
    await expect(getHookInstallJob('not-a-job', 'csrf')).rejects.toThrow(/job id/i);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
