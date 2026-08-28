import render from 'preact-render-to-string';
import { describe, expect, it, vi } from 'vitest';
import type { ManagedServer } from './control-api';
import type { DashboardData } from './dashboard-model';
import {
  canRunBackupMutation,
  importedServerPanelUrl,
  minecraftCreateSoftwareOptions,
  serverActionDescription,
  serverWorkloadIsRunning,
  ServersPage,
  supportsMarketplaceSoftware,
} from './servers';

const nativeServer: ManagedServer = {
  id: 'server-1',
  name: 'Survival',
  instanceName: 'helix-game-server-1',
  software: 'Paper',
  version: '1.21.8',
  status: 'online',
  panelRunning: true,
  startOnBoot: true,
  playersOnline: 3,
  maxPlayers: 20,
  cpuPercent: 12,
  memoryUsedMb: 2_048,
  memoryLimitMb: 4_096,
  tps: 20,
  managerPanelPort: 0,
  panelPort: 0,
  gamePort: 25_565,
  path: '/srv/helix/servers/server-1',
  warnings: [],
  manager: 'helix',
  executionBackend: 'docker',
  appearance: { kind: 'default', revision: 0 },
};

const data: DashboardData = {
  overview: { data: null, phase: 'ready', error: null },
  inventory: { data: null, phase: 'ready', error: null },
  servers: { data: [], phase: 'ready', error: null },
  integration: { data: null, phase: 'ready', error: null },
  refresh: vi.fn(async () => undefined),
  isRefreshing: false,
  refreshIntervalMs: 1_000,
};

describe('Servers route', () => {
  it('retains the native creation entry point after code splitting', () => {
    const markup = render(<ServersPage data={data} csrfToken="csrf" canManageServers canManageBackups canManageNetwork onSessionExpired={() => undefined} />);

    expect(markup).toContain('New server');
    expect(markup).toContain('Native game hosting');
    expect(markup).toContain('external managers remain separate');
  });

  it('offers every currently installable native Minecraft software', () => {
    expect(minecraftCreateSoftwareOptions.map((option) => option.id)).toEqual([
      'paper', 'purpur', 'folia', 'fabric', 'vanilla',
    ]);
  });

  it('only exposes the marketplace for supported Helix-native software', () => {
    expect(['Paper', 'Purpur', 'Folia', 'Fabric'].every(supportsMarketplaceSoftware)).toBe(true);
    expect(['Vanilla', 'NeoForge', 'AMP'].some(supportsMarketplaceSoftware)).toBe(false);
  });

  it('keeps server inventory visible while disabling mutations for view-only users', () => {
    const readonlyData: DashboardData = {
      ...data,
      servers: { data: [nativeServer], phase: 'ready', error: null },
    };
    const markup = render(<ServersPage data={readonlyData} csrfToken="csrf" canManageServers={false} canManageBackups={false} canManageNetwork={false} onSessionExpired={() => undefined} />);

    expect(markup).toContain('Survival');
    expect(markup).toContain('Open');
    expect(markup).toContain('New server');
    expect(markup).toContain('disabled');
    expect(markup.match(/Requires games\.manage permission/gu)?.length).toBeGreaterThanOrEqual(4);
  });

  it('maps backup controls to the backend capability split', () => {
    expect(canRunBackupMutation('create', true, false, true)).toBe(true);
    expect(canRunBackupMutation('create', false, true, true)).toBe(false);
    expect(canRunBackupMutation('restore', false, true, false)).toBe(true);
    expect(canRunBackupMutation('restore', true, false, true)).toBe(false);
    expect(canRunBackupMutation('trash', true, true, true)).toBe(true);
    expect(canRunBackupMutation('undo', true, true, false)).toBe(false);
  });

  it('describes AMP lifecycle and native update safety without overclaiming', () => {
    const ampServer: ManagedServer = { ...nativeServer, manager: 'amp_import', executionBackend: 'external' };
    expect(serverActionDescription(ampServer, 'start')).toContain('ask AMP to start');
    expect(serverActionDescription(ampServer, 'restart')).toContain('wait for AMP to report the instance online');
    expect(serverActionDescription(ampServer, 'restart')).not.toContain('Minecraft health check');

    expect(serverActionDescription(nativeServer, 'update')).toContain('restart and health-check Minecraft');
    expect(serverActionDescription({ ...nativeServer, status: 'offline' }, 'update')).toContain('keeping this server stopped');
    expect(serverActionDescription({ ...nativeServer, status: 'offline' }, 'update')).toContain('not automatically health-validated or rolled back');
    expect(serverActionDescription(nativeServer, 'kill')).toContain('SIGKILL');
    expect(serverActionDescription(nativeServer, 'kill')).toContain('when Stop is stuck');
    expect(serverActionDescription(ampServer, 'kill')).toContain('cannot force-kill AMP');
  });

  it('offers native Kill when the container is up, including hung Stop', () => {
    const onlineData: DashboardData = {
      ...data,
      servers: { data: [nativeServer], phase: 'ready', error: null },
    };
    const hung: ManagedServer = { ...nativeServer, status: 'offline', panelRunning: true };
    const hungData: DashboardData = {
      ...data,
      servers: { data: [hung], phase: 'ready', error: null },
    };
    const stopped: ManagedServer = { ...nativeServer, status: 'offline', panelRunning: false };
    const stoppedData: DashboardData = {
      ...data,
      servers: { data: [stopped], phase: 'ready', error: null },
    };
    const ampServer: ManagedServer = {
      ...nativeServer,
      manager: 'amp_import',
      executionBackend: 'external',
    };
    const ampData: DashboardData = {
      ...data,
      servers: { data: [ampServer], phase: 'ready', error: null },
    };

    expect(serverWorkloadIsRunning(nativeServer)).toBe(true);
    expect(serverWorkloadIsRunning(hung)).toBe(true);
    expect(serverWorkloadIsRunning(stopped)).toBe(false);
    expect(serverWorkloadIsRunning(ampServer)).toBe(true);

    const onlineMarkup = render(<ServersPage data={onlineData} csrfToken="csrf" canManageServers canManageBackups canManageNetwork onSessionExpired={() => undefined} />);
    expect(onlineMarkup).toContain('Kill');
    expect(onlineMarkup).toContain('Stop');

    const hungMarkup = render(<ServersPage data={hungData} csrfToken="csrf" canManageServers canManageBackups canManageNetwork onSessionExpired={() => undefined} />);
    expect(hungMarkup).toContain('Kill');
    expect(hungMarkup).toContain('Restart');
    expect(hungMarkup).not.toContain('m8 5 11 7-11 7V5Z');

    const stoppedMarkup = render(<ServersPage data={stoppedData} csrfToken="csrf" canManageServers canManageBackups canManageNetwork onSessionExpired={() => undefined} />);
    expect(stoppedMarkup).toContain('Start');
    expect(stoppedMarkup).toContain('m8 5 11 7-11 7V5Z');
    expect(stoppedMarkup).not.toContain('Kill');

    const ampMarkup = render(<ServersPage data={ampData} csrfToken="csrf" canManageServers canManageBackups canManageNetwork onSessionExpired={() => undefined} />);
    expect(ampMarkup).toContain('Stop');
    expect(ampMarkup).not.toContain('Kill');
  });

  it('builds AMP deep links from the manager port and opaque instance identity', () => {
    const ampServer: ManagedServer = {
      ...nativeServer,
      id: 'amp:a1b2c3d4-1111-4222-8333-123456789abc',
      manager: 'amp_import',
      managerPanelPort: 8_080,
      executionBackend: 'external',
    };
    expect(importedServerPanelUrl(ampServer, '192.0.2.10')).toBe('http://192.0.2.10:8080/instances/a1b2c3d4');
    expect(importedServerPanelUrl(ampServer, 'fd7a:115c:a1e0::1')).toBe('http://[fd7a:115c:a1e0::1]:8080/instances/a1b2c3d4');
    expect(importedServerPanelUrl({ ...ampServer, managerPanelPort: 0 }, '192.0.2.10')).toBeNull();
    expect(importedServerPanelUrl({ ...ampServer, id: 'amp:../../admin' }, '192.0.2.10')).toBeNull();
    expect(importedServerPanelUrl(ampServer, 'host.example/path')).toBeNull();
  });
});
