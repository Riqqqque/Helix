import { afterEach, describe, expect, it, vi } from 'vitest';
import { getServerManagerReadiness, parseServerManagerReadiness } from './server-manager-api';

const entry = (id: string, installable: boolean) => ({
  id,
  name: id === 'paper' ? 'Paper' : 'NeoForge',
  kind: id === 'paper' ? 'plugin_server' : 'mod_server',
  status: installable ? 'ready' : 'validation_pending',
  installable,
  recommended: id === 'paper',
  appeal: 'Useful for this kind of server.',
  note: installable ? 'Ready now.' : 'Validation is still in progress.',
});

const readiness = {
  schema_version: 1,
  availability: 'ready',
  manager: 'helix',
  execution_backend: 'docker',
  backend_version: '27.0.0',
  supported_games: ['minecraft'],
  supported_minecraft_software: ['paper'],
  minecraft_software_catalog: [entry('paper', true), entry('neoforge', false)],
  features: ['verified_downloads'],
  console_history_retention_bytes: 67_108_864,
  console_history_retention_files: 8,
  backup_trash_retention_days: 30,
  instance_root: '/not-exposed-by-ui',
  backup_root: '/not-exposed-by-ui',
  collected_at_unix_ms: 1_800_000_000_000,
};

afterEach(() => vi.unstubAllGlobals());

describe('native server manager readiness', () => {
  it('keeps installable software separate from explained unavailable choices', () => {
    const parsed = parseServerManagerReadiness(readiness);
    expect(parsed.availability).toBe('ready');
    if (parsed.availability === 'ready') {
      expect(parsed.supportedMinecraftSoftware).toEqual(['paper']);
      expect(parsed.minecraftSoftwareCatalog[1]).toMatchObject({ id: 'neoforge', installable: false, status: 'validation_pending' });
    }
  });

  it('rejects a catalog that claims a pending choice is installable', () => {
    expect(() => parseServerManagerReadiness({ ...readiness, minecraft_software_catalog: [{ ...entry('neoforge', false), installable: true }] })).toThrow(/inconsistent/i);
  });

  it('parses the broker-unavailable gate without fabricating choices', () => {
    expect(parseServerManagerReadiness({
      schema_version: 1,
      availability: 'unavailable',
      available_features: [],
      blockers: [{ code: 'privileged_broker', status: 'required' }],
      collected_at_unix_ms: 1_800_000_000_000,
    })).toMatchObject({ availability: 'unavailable', supportedMinecraftSoftware: [], minecraftSoftwareCatalog: [] });
  });

  it('loads the manager-specific readiness route with CSRF', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify(readiness), { status: 200, headers: { 'Content-Type': 'application/json' } }));
    vi.stubGlobal('fetch', fetchMock);
    await getServerManagerReadiness('csrf');
    expect(fetchMock.mock.calls[0]?.[0]).toBe('/api/v1/servers/manager/readiness');
    expect((fetchMock.mock.calls[0]?.[1] as RequestInit).headers).toMatchObject({ 'X-Helix-CSRF': 'csrf' });
  });
});
