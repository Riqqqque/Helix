import { afterEach, describe, expect, it, vi } from 'vitest';
import { migrateServer, migrateServerPreflight, parseMigratePreflight } from './server-migrate-api';

const preflight = {
  schema_version: 1,
  game: 'minecraft',
  source_kind: 'amp',
  source_id: 'amp:71b629b7-5861-47b8-907b-acde40dadc9e',
  source_name: 'Survival',
  source_path: '/home/amp/.ampdata/instances/Survival01',
  game_root: '/home/amp/.ampdata/instances/Survival01/Minecraft',
  software: 'paper',
  terraria_software: null,
  copy_server_jar: false,
  version: 'latest',
  version_used_latest: true,
  memory_mb: 4_096,
  max_players: 20,
  running: true,
  status: 'online',
  files: 12,
  bytes: 48_000_000,
  skipped: 4,
  copies: ['world/level.dat', 'plugins/WorldGuard.jar'],
  skips: ['logs', 'MinecraftModule.kvp'],
  warning: null,
  blockers: ['Stop the source server first. A live world copy can miss chunks or lock files.'],
  notes: ['Helix copies into a new native server. AMP and Pterodactyl files are not edited or deleted.'],
};

describe('server migrate API', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('parses a Minecraft AMP preflight', () => {
    const parsed = parseMigratePreflight(preflight);
    expect(parsed.game).toBe('minecraft');
    expect(parsed.sourceKind).toBe('amp');
    expect(parsed.software).toBe('paper');
    expect(parsed.running).toBe(true);
    expect(parsed.copies).toContain('world/level.dat');
    expect(parsed.blockers[0]).toContain('Stop the source server first');
  });

  it('parses a folder Terraria preflight', () => {
    const parsed = parseMigratePreflight({
      ...preflight,
      game: 'terraria',
      source_kind: 'folder',
      source_id: '/var/lib/pterodactyl/volumes/abcd',
      software: null,
      terraria_software: 'tmodloader',
      running: false,
      status: 'folder',
      blockers: [],
    });
    expect(parsed.game).toBe('terraria');
    expect(parsed.terrariaSoftware).toBe('tmodloader');
    expect(parsed.software).toBeNull();
  });

  it('posts inspect and copy requests', async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith('/migrate/preflight')) {
        return new Response(JSON.stringify(preflight), { status: 200 });
      }
      return new Response(JSON.stringify({ job_id: '12345678-1234-4234-8234-123456789abc' }), {
        status: 200,
      });
    });
    vi.stubGlobal('fetch', fetchMock);

    const inspected = await migrateServerPreflight(
      { kind: 'amp', instance_id: 'amp:71b629b7-5861-47b8-907b-acde40dadc9e' },
      'csrf',
    );
    expect(inspected.sourceName).toBe('Survival');

    const job = await migrateServer(
      {
        source: { kind: 'amp', instance_id: 'amp:71b629b7-5861-47b8-907b-acde40dadc9e' },
        name: 'Survival Helix',
        game: 'minecraft',
        software: 'paper',
        memory_mb: 4_096,
        max_players: 20,
        network_exposure: 'private',
        start_on_boot: true,
        eula_accepted: true,
        source_stopped: true,
        copy_acknowledged: true,
      },
      'csrf',
    );
    expect(job.jobId).toBe('12345678-1234-4234-8234-123456789abc');
    expect(String(fetchMock.mock.calls[0]?.[0])).toContain('/api/v1/servers/migrate/preflight');
    expect(String(fetchMock.mock.calls[1]?.[0])).toContain('/api/v1/servers/migrate');
  });
});
