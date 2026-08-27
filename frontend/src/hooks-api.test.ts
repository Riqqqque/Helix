import { afterEach, describe, expect, it, vi } from 'vitest';
import { getHookInventory, manageHookService, parseHookInventory } from './hooks-api';

const inventory = {
  schema_version: 1,
  hooks: [
    { id: 'plex', kind: 'systemd', unit: 'plexmediaserver.service', installed: true, active: true, active_state: 'active', enabled: true, enabled_state: 'enabled', controllable: true, actions: ['start', 'stop', 'restart', 'enable', 'disable'], panel_port: null, instance_count: null, unverified_instance_count: null, error: null },
    { id: 'amp', kind: 'api', installed: true, active: true, active_state: 'connected', enabled: true, enabled_state: 'configured', controllable: false, actions: [], panel_port: 8080, instance_count: 4, unverified_instance_count: 0, error: null },
  ],
  collected_at_unix_ms: 1_800_000_000_000,
};

afterEach(() => vi.unstubAllGlobals());

describe('hooks API', () => {
  it('keeps systemd control and external API connections distinct', () => {
    const parsed = parseHookInventory(inventory);
    expect(parsed.hooks[0]).toMatchObject({ id: 'plex', kind: 'systemd', controllable: true });
    expect(parsed.hooks[1]).toMatchObject({ id: 'amp', kind: 'api', panelPort: 8080, instanceCount: 4 });
  });

  it('rejects duplicate identities and invented actions', () => {
    expect(() => parseHookInventory({ ...inventory, hooks: [inventory.hooks[0], inventory.hooks[0]] })).toThrow();
    expect(() => parseHookInventory({ ...inventory, hooks: [{ ...inventory.hooks[0], actions: ['reboot-host'] }] })).toThrow();
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
});
