import { describe, expect, it } from 'vitest';

describe('server-scoped file browsing', () => {
  it('shows the server name instead of its host path and never climbs above it', async () => {
    const { fileCrumbs, isWithinRoot } = await import('./file-manager');
    const root = { path: '/NVME/helix/instances/abc', label: 'TestServer' };
    expect(fileCrumbs('/NVME/helix/instances/abc', root)).toEqual([['TestServer', '/NVME/helix/instances/abc']]);
    expect(fileCrumbs('/NVME/helix/instances/abc/plugins/x', root)).toEqual([
      ['TestServer', '/NVME/helix/instances/abc'],
      ['plugins', '/NVME/helix/instances/abc/plugins'],
      ['x', '/NVME/helix/instances/abc/plugins/x'],
    ]);
    expect(isWithinRoot('/NVME/helix/instances', root.path)).toBe(false);
    expect(isWithinRoot('/NVME/helix/instances/abcdef', root.path)).toBe(false);
    expect(isWithinRoot('/NVME/helix/instances/abc/mods', root.path)).toBe(true);
    expect(fileCrumbs('/srv/data')).toEqual([['/', '/'], ['srv', '/srv'], ['data', '/srv/data']]);
  });
});
