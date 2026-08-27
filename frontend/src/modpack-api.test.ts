import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  createMinecraftModpack,
  getModpackProject,
  parseMinecraftModpackCreateResult,
  parseModpackProjectDetail,
  parseModpackSearchPage,
  searchModpacks,
} from './modpack-api';

const csrf = 'MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM';
const searchResponse = {
  schema_version: 1,
  query: '',
  offset: 0,
  limit: 12,
  total_hits: 2,
  results: [{
    project_id: 'fabricPack1',
    slug: 'fabric-adventure',
    title: 'Fabric Adventure',
    description: 'A server-capable adventure pack.',
    author: 'Rique',
    downloads: 12000,
    follows: 500,
    latest_version: '1.2.0',
    minecraft_versions: ['1.21.1'],
    loaders: ['fabric'],
    server_side: 'required',
    compatibility_status: 'fabric_candidate',
    compatibility_reason: 'Fabric is lifecycle-ready; choose a stable server-capable release to continue',
    requires_version_check: true,
    web_url: 'https://modrinth.com/modpack/fabric-adventure',
    icon_url: '/api/v1/marketplace/modrinth/image?path=%2Fdata%2FfabricPack1%2Ficon.png',
  }, {
    project_id: 'neoPack22',
    slug: 'neoforge-adventure',
    title: 'NeoForge Adventure',
    description: null,
    author: null,
    downloads: 500,
    follows: 20,
    latest_version: null,
    minecraft_versions: ['1.21.1'],
    loaders: ['neoforge'],
    server_side: 'required',
    compatibility_status: 'incompatible',
    compatibility_reason: 'NeoForge packs are preview-only until Helix has a lifecycle-ready NeoForge server loader',
    requires_version_check: false,
    web_url: 'https://modrinth.com/modpack/neoforge-adventure',
    icon_url: null,
  }],
};

const projectResponse = {
  schema_version: 1,
  project: {
    id: 'fabricPack1',
    slug: 'fabric-adventure',
    title: 'Fabric Adventure',
    description: 'A server-capable adventure pack.',
    body: 'Plain text project details.',
    author: null,
    downloads: 12000,
    followers: 500,
    server_side: 'required',
    client_side: 'required',
    loaders: ['fabric'],
    web_url: 'https://modrinth.com/modpack/fabric-adventure',
    icon_url: '/api/v1/marketplace/modrinth/image?path=%2Fdata%2FfabricPack1%2Ficon.png',
  },
  versions: [{
    id: 'release123',
    name: 'Fabric Adventure 1.2.0',
    version_number: '1.2.0',
    version_type: 'release',
    status: 'listed',
    date_published: '2026-08-20T10:30:00Z',
    downloads: 2000,
    game_versions: ['1.21.1'],
    loaders: ['fabric'],
    installable: true,
    compatibility_reason: 'Stable Fabric server pack',
    mrpack_file: {
      filename: 'fabric-adventure-1.2.0.mrpack',
      size: 1000000,
      modrinth_declared_sha512_available: true,
    },
  }, {
    id: 'forge1234',
    name: 'Forge Adventure',
    version_number: '2.0.0',
    version_type: 'release',
    status: 'listed',
    date_published: '2026-08-21T10:30:00Z',
    downloads: 100,
    game_versions: ['1.21.1'],
    loaders: ['forge'],
    installable: false,
    compatibility_reason: 'Forge packs are preview-only until Helix has a lifecycle-ready Forge server loader',
    mrpack_file: null,
  }],
  compatible_version_count: 1,
  version_results_truncated: false,
  installation_scope: {
    loader: 'fabric',
    stable_releases_only: true,
    modrinth_declared_sha512_required: true,
    server_optional_files: 'excluded',
    client_only_files: 'excluded',
    exact_exclusion_counts: 'reported_after_archive_validation',
    full_pack_parity: false,
  },
};

const resultResponse = {
  schema_version: 1,
  instance_id: 'helix:id',
  modpack: {
    project_title: 'Fabric Adventure',
    version_number: '1.2.0',
    minecraft_version: '1.21.1',
    fabric_loader_version: '0.16.14',
    installed_server_files: 42,
    excluded_server_optional_files: 3,
    excluded_client_only_files: 8,
    server_safe_subset: true,
    full_pack_parity: false,
  },
  modrinth_declared_sha512_verified: true,
  declared_file_hashes_verified: ['sha1', 'sha512'],
};

afterEach(() => vi.unstubAllGlobals());

describe('Modrinth modpack contracts', () => {
  it('parses Fabric candidates while preserving precise incompatible loader reasons', () => {
    const search = parseModpackSearchPage(searchResponse);
    const project = parseModpackProjectDetail(projectResponse);

    expect(search.query).toBe('');
    expect(search.results[0]).toMatchObject({ compatibilityStatus: 'fabric_candidate', requiresVersionCheck: true, iconUrl: expect.stringContaining('/marketplace/modrinth/image?') });
    expect(search.results[1]?.compatibilityReason).toContain('NeoForge');
    expect(project.versions[0]).toMatchObject({ installable: true, versionType: 'release' });
    expect(project.versions[1]).toMatchObject({ installable: false, mrpackFile: null });
  });

  it('rejects unsafe external links and installable versions without an mrpack', () => {
    expect(() => parseModpackSearchPage({
      ...searchResponse,
      results: [{ ...searchResponse.results[0], web_url: 'https://example.com/modpack/fabric-adventure' }],
    })).toThrow(/Modrinth URL/i);
    expect(() => parseModpackProjectDetail({
      ...projectResponse,
      versions: [{ ...projectResponse.versions[0], mrpack_file: null }],
      compatible_version_count: 1,
    })).toThrow(/omitted its .mrpack/i);
  });

  it('parses exact excluded-file counts and refuses inconsistent safety claims', () => {
    expect(parseMinecraftModpackCreateResult(resultResponse)).toMatchObject({
      installedServerFiles: 42,
      excludedServerOptionalFiles: 3,
      excludedClientOnlyFiles: 8,
      fullPackParity: false,
    });
    expect(() => parseMinecraftModpackCreateResult({
      ...resultResponse,
      modpack: { ...resultResponse.modpack, full_pack_parity: true },
    })).toThrow(/inconsistent safety/i);
  });

  it('uses only search, project, and opaque project/version create contracts', async () => {
    const responses = [searchResponse, projectResponse, { job_id: 'job12345', reused: false }];
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(new Response(JSON.stringify(responses.shift()), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    })));
    vi.stubGlobal('fetch', fetchMock);

    await searchModpacks(' fabric adventure ', 0, 12, csrf);
    await getModpackProject('fabricPack1', csrf);
    await createMinecraftModpack({
      name: 'Adventure',
      memory_mb: 6144,
      max_players: 20,
      game_port: 25565,
      start_on_boot: true,
      eula_accepted: true,
      project_id: 'fabricPack1',
      version_id: 'release123',
    }, csrf);

    expect(fetchMock.mock.calls.map((call) => call[0])).toEqual([
      '/api/v1/servers/minecraft/modpacks/search?query=+fabric+adventure+&offset=0&limit=12',
      '/api/v1/servers/minecraft/modpacks/projects/fabricPack1',
      '/api/v1/servers/minecraft/modpacks',
    ]);
    const post = fetchMock.mock.calls[2]?.[1] as RequestInit;
    expect(JSON.parse(String(post.body))).toEqual({
      name: 'Adventure',
      memory_mb: 6144,
      max_players: 20,
      game_port: 25565,
      start_on_boot: true,
      eula_accepted: true,
      project_id: 'fabricPack1',
      version_id: 'release123',
    });
    expect(String(post.body)).not.toContain('url');
    expect(String(post.body)).not.toContain('path');
    expect(String(post.body)).not.toContain('loader');
  });
});
