import render from 'preact-render-to-string';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ManagedServer } from './control-api';
import type { DashboardData } from './dashboard-model';
import {
  allocatedMemoryOptions,
  ampHelpPanelUrl,
  canRunBackupMutation,
  importedServerPanelUrl,
  joinErrorOffersPortChange,
  parseAmpPortClaim,
  minecraftCreateSoftwareOptions,
  NewServerChooser,
  publicInternetHint,
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
  playerCountVerified: true,
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
  kind: 'minecraft',
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
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('retains the native creation entry point after code splitting', () => {
    const markup = render(<ServersPage data={data} csrfToken="csrf" canManageServers canManageBackups canManageNetwork onSessionExpired={() => undefined} />);

    expect(markup).toContain('New server');
    expect(markup).toContain('Native game hosting');
    expect(markup).toContain('external managers remain separate');
  });

  it('opens a native server from the URL fragment', () => {
    vi.stubGlobal('window', {
      location: { hash: '#servers/server-1' },
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    });
    const markup = render(
      <ServersPage
        data={{ ...data, servers: { data: [nativeServer], phase: 'ready', error: null } }}
        csrfToken="csrf"
        canManageServers
        canManageBackups
        canManageNetwork
        onSessionExpired={() => undefined}
      />,
    );

    expect(markup).toContain('Opening Survival');
  });

  it('shows Minecraft and V Rising marks in the new-server chooser', () => {
    const markup = render(
      <NewServerChooser
        onMinecraft={() => undefined}
        onVRising={() => undefined}
        onValheim={() => undefined}
        onTerraria={() => undefined}
        onClose={() => undefined}
      />,
    );

    expect(markup).toContain('Minecraft: Java Edition');
    expect(markup).toContain('dialog-body');
    expect(markup).toContain('V Rising');
    expect(markup).toContain('Valheim');
    expect(markup).toContain('Terraria');
    expect(markup).toContain('Click to install');
    expect(markup).toContain('game-mark--minecraft');
    expect(markup).toContain('game-mark--vrising');
    expect(markup).not.toContain('Not available on Linux');
  });

  it('lists native V Rising servers beside Minecraft', () => {
    const vrising: ManagedServer = {
      ...nativeServer,
      id: 'server-vr',
      name: 'Castle',
      instanceName: 'helix-game-server-vr',
      software: 'V Rising',
      version: 'dedicated',
      kind: 'vrising',
      gamePort: 9_876,
      playersOnline: 0,
      playerCountVerified: false,
      tps: null,
    };
    const markup = render(
      <ServersPage
        data={{ ...data, servers: { data: [nativeServer, vrising], phase: 'ready', error: null } }}
        csrfToken="csrf"
        canManageServers
        canManageBackups
        canManageNetwork
        onSessionExpired={() => undefined}
      />,
    );

    expect(markup).toContain('Survival');
    expect(markup).toContain('Castle');
    expect(markup).toContain('V Rising');
    expect(markup).toContain('3 / 20');
    expect(markup).not.toContain('0 / 20');
  });

  it('offers every currently installable native Minecraft software', () => {
    expect(minecraftCreateSoftwareOptions.map((option) => option.id)).toEqual([
      'paper', 'purpur', 'folia', 'leaves', 'fabric', 'neoforge', 'forge', 'quilt', 'pufferfish', 'vanilla',
    ]);
  });

  it('only exposes the marketplace for supported Helix-native software', () => {
    expect(['Paper', 'Purpur', 'Folia', 'Leaves', 'Fabric', 'Forge', 'NeoForge', 'Quilt', 'Pufferfish'].every(supportsMarketplaceSoftware)).toBe(true);
    expect(['Vanilla', 'AMP'].some(supportsMarketplaceSoftware)).toBe(false);
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
    expect(canRunBackupMutation('create', true, false)).toBe(true);
    expect(canRunBackupMutation('create', false, true)).toBe(false);
    expect(canRunBackupMutation('restore', false, true)).toBe(true);
    expect(canRunBackupMutation('restore', true, false)).toBe(false);
    expect(canRunBackupMutation('trash', true, true)).toBe(true);
    expect(canRunBackupMutation('undo', true, true)).toBe(true);
    expect(canRunBackupMutation('trash', true, false)).toBe(false);
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
    const vrising: ManagedServer = { ...nativeServer, kind: 'vrising' };
    expect(serverActionDescription(vrising, 'restart')).toContain('ready marker');
    expect(serverActionDescription(vrising, 'restart')).not.toContain('Minecraft health check');
    expect(serverActionDescription(vrising, 'start')).toContain('ready marker');
  });

  it('treats AMP idle as asleep, with Start instead of Restart', () => {
    const idleAmp: ManagedServer = {
      ...nativeServer,
      name: 'AllTheMons',
      instanceName: 'AllTheMons01',
      status: 'idle',
      panelRunning: true,
      memoryUsedMb: 0,
      memoryLimitMb: 10_240,
      manager: 'amp_import',
      executionBackend: 'external',
    };
    expect(serverWorkloadIsRunning(idleAmp)).toBe(false);
    expect(serverActionDescription(idleAmp, 'start')).toContain('wake');
    expect(serverActionDescription(idleAmp, 'start')).toContain('sleeping');

    const markup = render(
      <ServersPage
        data={{ ...data, servers: { data: [idleAmp], phase: 'ready', error: null } }}
        csrfToken="csrf"
        canManageServers
        canManageBackups
        canManageNetwork
        onSessionExpired={() => undefined}
      />,
    );
    expect(markup).toContain('Idle');
    expect(markup).toContain('Start');
    expect(markup).not.toContain('Restart');
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

  it('lists allocated memory steps inside each game bound', () => {
    expect(allocatedMemoryOptions('minecraft', 4096)).toEqual([
      1024, 2048, 4096, 6144, 8192, 12288, 16384, 24576,
    ]);
    expect(allocatedMemoryOptions('vrising', 4096)[0]).toBe(2048);
    expect(allocatedMemoryOptions('valheim', 4096)).not.toContain(24576);
    expect(allocatedMemoryOptions('terraria', 512)[0]).toBe(512);
    expect(allocatedMemoryOptions('minecraft', 3072)).toContain(3072);
  });

  it('tells operators how public Direct Connect still needs the game ports', () => {
    expect(publicInternetHint('minecraft', 25565, null)).toContain('TCP 25565');
    expect(publicInternetHint('terraria', 7777, null)).toContain('TCP 7777');
    expect(publicInternetHint('vrising', 9876, 9877)).toContain('UDP 9876 and 9877');
    expect(publicInternetHint('valheim', 2456, null)).toContain('UDP 2456–2458');
    expect(publicInternetHint('vrising', 9876, 9877, true)).toContain('UPnP');
    expect(publicInternetHint('minecraft', 25565, null, true)).toMatch(/scanners/i);
  });

  it('points Join port conflicts at Settings and names AMP mappings', () => {
    expect(joinErrorOffersPortChange('AMP already has port 25566 claimed')).toBe(true);
    expect(joinErrorOffersPortChange('Leftover AMP router mapping on port 25566. No AMP instance currently lists that port in its files.')).toBe(true);
    expect(joinErrorOffersPortChange('router TCP port 25566 already has a mapping; Helix will not overwrite an unowned router rule')).toBe(true);
    expect(joinErrorOffersPortChange('the router reports CGNAT address 100.64.1.1')).toBe(false);
  });

  it('parses live AMP claims and leftover UPnP forwards for the create help card', () => {
    expect(parseAmpPortClaim('AMP already has port 25566 claimed. Held by Survival01 (game port).')).toEqual({
      port: 25566,
      leftover: false,
    });
    expect(parseAmpPortClaim('Leftover AMP router mapping on port 25566. No AMP instance currently lists that port in its files.')).toEqual({
      port: 25566,
      leftover: true,
    });
    expect(parseAmpPortClaim('game port 25565 is already assigned to another Helix server')).toBeNull();
    const ampServer: ManagedServer = {
      ...nativeServer,
      id: 'amp:a1b2c3d4-1111-4222-8333-123456789abc',
      name: 'Survival01',
      instanceName: 'Survival01',
      manager: 'amp_import',
      managerPanelPort: 8_080,
      executionBackend: 'external',
    };
    expect(ampHelpPanelUrl('Held by Survival01 (game port)', [ampServer], '192.0.2.10')).toBe(
      'http://192.0.2.10:8080/instances/a1b2c3d4',
    );
    expect(ampHelpPanelUrl('AMP already has port 25566 claimed', [ampServer], '192.0.2.10')).toBe(
      'http://192.0.2.10:8080',
    );
  });

  it('shows a native TPS sample on the list and an em dash when it is missing', () => {
    const onlineData: DashboardData = {
      ...data,
      servers: { data: [nativeServer], phase: 'ready', error: null },
    };
    const missingData: DashboardData = {
      ...data,
      servers: { data: [{ ...nativeServer, tps: null }], phase: 'ready', error: null },
    };
    const onlineMarkup = render(<ServersPage data={onlineData} csrfToken="csrf" canManageServers canManageBackups canManageNetwork onSessionExpired={() => undefined} />);
    const missingMarkup = render(<ServersPage data={missingData} csrfToken="csrf" canManageServers canManageBackups canManageNetwork onSessionExpired={() => undefined} />);

    expect(onlineMarkup).toContain('20.0');
    expect(missingMarkup).toContain('TPS');
    expect(missingMarkup).not.toContain('20.0');
  });

  it('offers Forget here for a hidden AMP connection without claiming AMP deletion', () => {
    const ampId = 'amp:71b629b7-5861-47b8-907b-acde40dadc9e';
    const store = new Map<string, string>([
      ['helix.servers.hidden-imports', JSON.stringify([ampId])],
    ]);
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => {
        store.set(key, value);
      },
    });
    const ampServer: ManagedServer = {
      ...nativeServer,
      id: ampId,
      name: 'AllTheMons',
      manager: 'amp_import',
      executionBackend: 'external',
    };
    const markup = render(
      <ServersPage
        data={{ ...data, servers: { data: [ampServer], phase: 'ready', error: null } }}
        csrfToken="csrf"
        canManageServers
        canManageBackups
        canManageNetwork
        onSessionExpired={() => undefined}
      />,
    );
    expect(markup).toContain('Removed and hidden');
    expect(markup).toContain('Hidden AMP connection');
    expect(markup).toContain('Forget here');
    expect(markup).toContain('Show again');
    expect(markup).not.toContain('Delete forever');
  });
});
