import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  getMinecraftPortPolicy,
  saveMinecraftPortPolicy,
  saveVRisingPortPolicy,
} from './port-policy-api';
import {
  getMinecraftVersions,
  getDirectory,
  getServerBackups,
  getServerDetail,
  parseDirectoryListing,
  parseHostInventory,
  parseMinecraftSettingsSaveResult,
  parseServers,
  restoreTrashedServerBackup,
  runServerAction,
  setNativeMemory,
  setNativeStartOnBoot,
  setServerNetworkExposure,
  saveServerSettings,
  trashServerBackup,
  type MinecraftSettings,
} from './control-api';

afterEach(() => vi.unstubAllGlobals());

describe('file manager API', () => {
  const listing = {
    path: '/HDD10tb1',
    parent: '/',
    writable: true,
    omitted_entries: 0,
    total_entries: 76,
    next_cursor: 'movie 050.mkv',
    has_more: true,
    page_limit: 50,
    entries: [{
      name: 'movie 001.mkv',
      path: '/HDD10tb1/movie 001.mkv',
      kind: 'file',
      size_bytes: 8_589_934_592,
      modified_unix_ms: 1_800_000_000_000,
      permissions: '0660',
      owner_uid: 1000,
      owner_gid: 1000,
      writable: true,
      restricted: false,
      symlink_target: null,
    }],
  };

  it('parses bounded cursor pagination and rejects contradictory cursors', () => {
    expect(parseDirectoryListing(listing)).toMatchObject({
      totalEntries: 76,
      nextCursor: 'movie 050.mkv',
      hasMore: true,
      pageLimit: 50,
    });
    expect(() => parseDirectoryListing({ ...listing, has_more: false })).toThrow(/pagination/i);
    expect(() => parseDirectoryListing({ ...listing, entries: Array(51).fill(listing.entries[0]) })).toThrow();
  });

  it('sends paths, cursors, and page sizes as encoded query parameters', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify(listing), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);

    await getDirectory('/HDD10tb1/TV & Movies', 'csrf', 'movie 050.mkv', 50);

    const [path] = fetchMock.mock.calls[0] as [string, RequestInit];
    const url = new URL(path, 'http://helix.local');
    expect(url.pathname).toBe('/api/v1/files');
    expect(url.searchParams.get('path')).toBe('/HDD10tb1/TV & Movies');
    expect(url.searchParams.get('cursor')).toBe('movie 050.mkv');
    expect(url.searchParams.get('limit')).toBe('50');
  });
});

describe('Minecraft version catalog', () => {
  it('loads published releases for the selected software', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({
      schema_version: 1,
      software: 'paper',
      allows_latest: true,
      latest_version: '1.21.8',
      versions: ['1.21.8', '1.21.7', '1.21.4'],
    }), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);

    await expect(getMinecraftVersions('paper', 'csrf')).resolves.toEqual({
      software: 'paper',
      allowsLatest: true,
      latestVersion: '1.21.8',
      versions: ['1.21.8', '1.21.7', '1.21.4'],
    });
    const url = new URL(String(fetchMock.mock.calls[0]?.[0]), 'http://helix.local');
    expect(url.pathname).toBe('/api/v1/servers/minecraft/versions');
    expect(url.searchParams.get('software')).toBe('paper');
  });

  it('keeps custom JAR catalogs from advertising latest', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({
      schema_version: 1,
      software: 'custom',
      allows_latest: false,
      latest_version: '1.21.8',
      versions: ['1.21.8'],
    }), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);

    await expect(getMinecraftVersions('custom', 'csrf')).resolves.toMatchObject({
      software: 'custom',
      allowsLatest: false,
      latestVersion: '1.21.8',
    });
  });
});

const settings: MinecraftSettings = {
  expectedRevision: 'a'.repeat(64),
  motd: 'Survival',
  gameMode: 'survival',
  difficulty: 'hard',
  maxPlayers: 20,
  viewDistance: 12,
  simulationDistance: 8,
  playerIdleTimeout: 15,
  onlineMode: true,
  pvp: true,
  allowFlight: false,
  whiteList: true,
  enforceWhiteList: true,
  spawnProtection: 8,
  gamePort: 25565,
  memoryMb: 4096,
  restartBehavior: {
    activation: 'server_restart',
    restartRequiredFields: [
      'motd', 'game_mode', 'difficulty', 'max_players', 'view_distance',
      'simulation_distance', 'player_idle_timeout', 'online_mode', 'pvp',
      'allow_flight', 'white_list', 'enforce_white_list', 'spawn_protection',
      'game_port',
    ],
    message: 'Changes saved here take effect the next time Minecraft starts.',
  },
};

function wireSettings(value = settings): Record<string, unknown> {
  return {
    expected_revision: value.expectedRevision,
    motd: value.motd,
    game_mode: value.gameMode,
    difficulty: value.difficulty,
    max_players: value.maxPlayers,
    view_distance: value.viewDistance,
    simulation_distance: value.simulationDistance,
    player_idle_timeout: value.playerIdleTimeout,
    online_mode: value.onlineMode,
    pvp: value.pvp,
    allow_flight: value.allowFlight,
    white_list: value.whiteList,
    enforce_white_list: value.enforceWhiteList,
    spawn_protection: value.spawnProtection,
    game_port: value.gamePort,
    memory_mb: value.memoryMb,
    restart_behavior: {
      activation: value.restartBehavior.activation,
      restart_required_fields: value.restartBehavior.restartRequiredFields,
      message: value.restartBehavior.message,
    },
  };
}

describe('native server API', () => {
  it('reads and writes bounded Minecraft port pools and the exposure toggle', async () => {
    const response = {
      schema_version: 1,
      policy: {
        game: 'minecraft',
        ranges: [{ start: 25565, end: 25574 }],
        ports: [25600],
        auto_forward_on_create: true,
      },
      capacity: 11,
      assigned_ports: [25565],
      amp_claimed_ports: [25566],
      available_count: 10,
      next_available_port: 25600,
    };
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify(response), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify(response), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ enabled: true }), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);

    await expect(getMinecraftPortPolicy('csrf')).resolves.toMatchObject({
      nextAvailablePort: 25600,
      autoForwardOnCreate: true,
      availableCount: 10,
      ampClaimedPorts: [25566],
    });
    await saveMinecraftPortPolicy({
      ranges: [{ start: 25565, end: 25574 }],
      ports: [25600],
      autoForwardOnCreate: true,
    }, 'csrf');
    await setServerNetworkExposure('helix:server-id', true, 'csrf');

    const [, policyRequest] = fetchMock.mock.calls[1] as [string, RequestInit];
    expect(policyRequest.method).toBe('PUT');
    expect(JSON.parse(String(policyRequest.body))).toEqual({
      game: 'minecraft',
      ranges: [{ start: 25565, end: 25574 }],
      ports: [25600],
      auto_forward_on_create: true,
    });
    const [exposurePath, exposureRequest] = fetchMock.mock.calls[2] as [string, RequestInit];
    expect(exposurePath).toContain('helix%3Aserver-id/network');
    expect(JSON.parse(String(exposureRequest.body))).toEqual({ enabled: true });
  });

  it('never enables UPnP auto-forward when saving the V Rising pool', async () => {
    const response = {
      schema_version: 1,
      policy: {
        game: 'vrising',
        ranges: [{ start: 9876, end: 9910 }],
        ports: [],
        auto_forward_on_create: false,
      },
      capacity: 35,
      assigned_ports: [],
      amp_claimed_ports: [],
      available_count: 35,
      next_available_port: 9876,
    };
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify(response), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);

    await saveVRisingPortPolicy({
      ranges: [{ start: 9876, end: 9910 }],
      ports: [],
      autoForwardOnCreate: true,
    }, 'csrf');

    const [, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(JSON.parse(String(request.body))).toEqual({
      game: 'vrising',
      ranges: [{ start: 9876, end: 9910 }],
      ports: [],
      auto_forward_on_create: false,
    });
  });

  it('updates native start-on-boot without starting the server now', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ enabled: false }), { status: 200 }),
    );
    vi.stubGlobal('fetch', fetchMock);

    await expect(setNativeStartOnBoot('helix:server-id', false, 'csrf')).resolves.toEqual({ enabled: false });
    const [path, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(path).toContain('helix%3Aserver-id/start-on-boot');
    expect(request.method).toBe('PUT');
    expect(JSON.parse(String(request.body))).toEqual({ enabled: false });
  });

  it('updates native allocated memory without inventing a restart', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({
        changed: true,
        memory_mb: 8192,
        container_republished: true,
      }), { status: 200 }),
    );
    vi.stubGlobal('fetch', fetchMock);

    await expect(setNativeMemory('helix:server-id', 8192, 'csrf')).resolves.toEqual({
      changed: true,
      memoryMb: 8192,
      containerRepublished: true,
    });
    const [path, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(path).toContain('helix%3Aserver-id/memory');
    expect(request.method).toBe('PUT');
    expect(JSON.parse(String(request.body))).toEqual({ memory_mb: 8192 });
  });

  it('parses a background action job without waiting for the work', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ job_id: 'ccf645d5-7896-4659-bc71-6f177efb589d' }), { status: 200 }),
    );
    vi.stubGlobal('fetch', fetchMock);

    await expect(runServerAction('helix:server-id', 'backup', 'csrf')).resolves.toEqual({
      jobId: 'ccf645d5-7896-4659-bc71-6f177efb589d',
    });
    const [path, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(path).toContain('helix%3Aserver-id/actions');
    expect(request.method).toBe('POST');
  });

  it('posts kill as its own typed server action', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ job_id: '0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0' }), { status: 200 }),
    );
    vi.stubGlobal('fetch', fetchMock);

    await expect(runServerAction('helix:server-id', 'kill', 'csrf')).resolves.toEqual({
      jobId: '0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0',
    });
    const [, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(JSON.parse(String(request.body))).toEqual({ action: 'kill' });
  });

  it('sends the guarded settings revision and parses the committed version', async () => {
    const committed = { ...settings, expectedRevision: 'b'.repeat(64) };
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ changed: true, restart_required: true, changed_fields: ['motd'], settings: wireSettings(committed) }), { status: 200 }),
    );
    vi.stubGlobal('fetch', fetchMock);

    await expect(saveServerSettings('helix:server-id', settings, 'csrf')).resolves.toEqual({
      changed: true,
      restartRequired: true,
      changedFields: ['motd'],
      settings: committed,
      containerRepublished: false,
      exposureNote: null,
      exposureWarning: null,
    });
    const [, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(JSON.parse(String(request.body))).toMatchObject({
      expected_revision: 'a'.repeat(64),
      max_players: 20,
      enforce_white_list: true,
      game_port: 25565,
      memory_mb: 4096,
    });
  });

  it('preserves the backend no-op signal without inventing a restart', () => {
    expect(parseMinecraftSettingsSaveResult({
      changed: false,
      restart_required: false,
      changed_fields: [],
      settings: wireSettings(),
    })).toEqual({
      changed: false,
      restartRequired: false,
      changedFields: [],
      settings,
      containerRepublished: false,
      exposureNote: null,
      exposureWarning: null,
    });
  });

  it('parses recoverable backup trash and uses only opaque IDs for delete and undo', async () => {
    const trashId = '019d1234-5678-4abc-8def-123456789abc';
    const catalog = {
      instance_id: 'helix:server-id',
      backups: [{ id: '1787800010959', created_at_unix_ms: 1787800010959, size_bytes: 512, definition_present: true }],
      trash: [{
        trash_id: trashId,
        backup_id: '1787799939239',
        trashed_at_unix_ms: 1787800020000,
        undo_available: true,
        undo_expires_at_unix_ms: null,
        purge_eligible_at_unix_ms: 1790392020000,
        automatic_purge_enabled: false,
        size_bytes: 256,
        definition_present: true,
        path: '/must/not/reach/the/ui',
      }],
      trash_policy: {
        purge_after_days: 30,
        automatic_purge_enabled: false,
        note: 'Deleted backups stay recoverable until an explicit purge is requested.',
      },
    };
    const trashed = {
      instance_id: 'helix:server-id', backup_id: '1787800010959', trash_id: trashId,
      trashed_at_unix_ms: 1787800020000, undo_available: true,
      undo_expires_at_unix_ms: null, purge_eligible_at_unix_ms: 1790392020000,
      automatic_purge_enabled: false,
    };
    const restored = {
      instance_id: 'helix:server-id', backup_id: '1787800010959', trash_id: trashId,
      restored_at_unix_ms: 1787800030000, cleanup_pending: false,
    };
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify(catalog), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify(trashed), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify(restored), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);

    const parsed = await getServerBackups('helix:server-id', 'csrf');
    expect(parsed.trash[0]).not.toHaveProperty('path');
    expect(parsed.trashPolicy.note).toContain('recoverable');
    await trashServerBackup('helix:server-id', '1787800010959', 'csrf');
    await restoreTrashedServerBackup('helix:server-id', trashId, 'csrf');

    const [deletePath, deleteRequest] = fetchMock.mock.calls[1] as [string, RequestInit];
    expect(deletePath).toContain('/backups/1787800010959');
    expect(deletePath).not.toContain('/must/');
    expect(deleteRequest.method).toBe('DELETE');
    expect(deleteRequest.body).toBe('{}');
    const deleteHeaders = new Headers(deleteRequest.headers);
    expect(deleteHeaders.get('Content-Type')).toBe('application/json');
    expect(deleteHeaders.get('X-Helix-CSRF')).toBe('csrf');
    const [undoPath, undoRequest] = fetchMock.mock.calls[2] as [string, RequestInit];
    expect(undoPath).toContain(`/trash/${trashId}/restore`);
    expect(undoRequest.method).toBe('POST');
  });

  it('rejects malformed detail status instead of rendering invented state', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ status: 'mystery' }), { status: 200 }),
    );
    vi.stubGlobal('fetch', fetchMock);

    await expect(getServerDetail('helix:server-id', 'csrf')).rejects.toThrow();
  });
});

describe('server list API', () => {
  const server = {
    id: 'amp:71b629b7-5861-47b8-907b-acde40dadc9e',
    name: 'AllTheMons',
    instance_name: 'AllTheMons01',
    kind: 'imported',
    software: 'Paper',
    version: '1.21.8',
    status: 'idle',
    panel_running: true,
    start_on_boot: true,
    players_online: 0,
    player_count_verified: true,
    max_players: 10,
    cpu_percent: 0,
    memory_used_mb: 0,
    memory_limit_mb: 10_240,
    tps: null,
    manager_panel_port: 8080,
    panel_port: 8081,
    game_port: 25565,
    path: '/home/amp/.ampdata/instances/AllTheMons01',
    warnings: [],
    manager: 'amp_import',
    execution_backend: 'external',
    appearance: { kind: 'default', revision: 0 },
  };

  it('accepts AMP idle and other lifecycle statuses', () => {
    const parsed = parseServers([server]);
    expect(parsed).toHaveLength(1);
    expect(parsed[0]).toMatchObject({
      name: 'AllTheMons',
      status: 'idle',
      panelRunning: true,
      memoryUsedMb: 0,
      memoryLimitMb: 10_240,
      playerCountVerified: true,
    });
    expect(() => {
      const rest: Record<string, unknown> = { ...server };
      delete rest.player_count_verified;
      parseServers([rest]);
    }).toThrow(/player_count_verified/i);
    const starting = parseServers([{ ...server, status: 'starting' }]);
    expect(starting[0]?.status).toBe('starting');
    const failed = parseServers([{ ...server, status: 'failed' }]);
    expect(failed[0]?.status).toBe('failed');
    expect(() => parseServers([{ ...server, status: 'mystery' }])).toThrow(/status/i);
  });
});

describe('host inventory', () => {
  it('reads process count separately from kernel thread count', () => {
    const parsed = parseHostInventory({
      disks: [],
      mounts: [],
      interfaces: [],
      routes: [],
      listeners: [],
      services: [],
      processes: [],
      load_average: [0.4, 0.7, 0.6],
      process_count: 490,
      thread_count: 2261,
      cpu_model: 'AMD Ryzen',
      collected_at_unix_ms: 1,
    });
    expect(parsed.processCount).toBe(490);
    expect(parsed.threadCount).toBe(2261);
    expect(() => parseHostInventory({
      disks: [],
      mounts: [],
      interfaces: [],
      routes: [],
      listeners: [],
      services: [],
      processes: [],
      load_average: [0.4, 0.7, 0.6],
      process_count: 2261,
      cpu_model: 'AMD Ryzen',
      collected_at_unix_ms: 1,
    })).toThrow(/thread_count/i);
  });
});
