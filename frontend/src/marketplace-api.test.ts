import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  getMarketplaceInstallJob,
  getMarketplaceProject,
  installMarketplaceProject,
  marketplaceProfileForSoftware,
  marketplaceResponseMatchesServer,
  parseMarketplaceInstallJob,
  parseMarketplaceInstallResult,
  parseMarketplaceProjectDetail,
  parseMarketplaceSearchPage,
  searchMarketplace,
  type MarketplaceServerContext,
} from './marketplace-api';

const csrf = 'MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM';
const jobId = '6b8f95ce-9c58-4c4c-b232-627a29ca1c03';
const server: MarketplaceServerContext = {
  id: 'native_survival',
  name: 'Survival',
  software: 'Paper',
  minecraftVersion: '1.21.4',
  status: 'online',
};
const compatibility = {
  minecraft_version: '1.21.4',
  server_software: 'Paper',
  content_kind: 'plugin',
  accepted_loaders: ['paper', 'spigot', 'bukkit'],
  install_directory: 'plugins',
};
const searchPage = {
  schema_version: 1,
  instance_id: server.id,
  compatibility,
  total_hits: 21,
  offset: 0,
  limit: 20,
  hits: [{
    project_id: 'P7dR8mSH',
    slug: 'fabric-api',
    title: 'Fast Async WorldEdit',
    description: 'Edit worlds quickly.',
    author: 'EngineHub',
    project_type: 'mod',
    server_side: 'required',
    downloads: 42_000_000,
    follows: 12_000,
    latest_version: '2.13.1',
    date_modified: '2026-08-20T10:30:00Z',
    web_url: 'https://modrinth.com/plugin/fabric-api',
    icon_url: '/api/v1/marketplace/modrinth/image?path=%2Fdata%2FP7dR8mSH%2Ficon.png',
  }],
  collected_at_unix_ms: 1_800_000_000_000,
};
const projectDetail = {
  schema_version: 1,
  instance_id: server.id,
  compatibility,
  project: {
    id: 'P7dR8mSH',
    slug: 'fabric-api',
    title: 'Fast Async WorldEdit',
    description: 'Edit worlds quickly.',
    body: 'Fast world editing tools.\n\nRead the project documentation before changing a large world.',
    project_type: 'mod',
    content_kind: 'plugin',
    server_side: 'required',
    downloads: 42_000_000,
    followers: 12_000,
    license: 'GPL-3.0-only',
    source_url: 'https://github.com/EngineHub/FastAsyncWorldEdit',
    issues_url: 'https://github.com/EngineHub/FastAsyncWorldEdit/issues',
    wiki_url: null,
    web_url: 'https://modrinth.com/plugin/fabric-api',
    icon_url: '/api/v1/marketplace/modrinth/image?path=%2Fdata%2FP7dR8mSH%2Ficon.png',
  },
  versions: [{
    id: 'paper-release-2131',
    name: 'Paper 2.13.1',
    version_number: '2.13.1',
    version_type: 'release',
    date_published: '2026-08-20T10:30:00Z',
    downloads: 5_000_000,
    game_versions: ['1.21.4'],
    loaders: ['paper', 'spigot'],
    has_primary_file: true,
  }, {
    id: 'paper-beta-2140',
    name: 'Paper 2.14.0 beta',
    version_number: '2.14.0-beta.1',
    version_type: 'beta',
    date_published: '2026-08-23T10:30:00Z',
    downloads: 2_000,
    game_versions: ['1.21.4'],
    loaders: ['paper'],
    has_primary_file: true,
  }],
  version_count_returned: 2,
  version_results_truncated: false,
  body_format: 'plain_text',
  collected_at_unix_ms: 1_800_000_000_100,
};
const installResult = {
  schema_version: 1,
  instance_id: server.id,
  project_id: 'P7dR8mSH',
  project_slug: 'fabric-api',
  project_title: 'Fast Async WorldEdit',
  version_id: 'paper-release-2131',
  version_number: '2.13.1',
  installed_projects: [{
    project_id: 'P7dR8mSH',
    project_slug: 'fabric-api',
    project_title: 'Fast Async WorldEdit',
    version_id: 'paper-release-2131',
    version_number: '2.13.1',
    filename: 'FastAsyncWorldEdit-Paper-2.13.1.jar',
  }, {
    project_id: 'required-dependency',
    project_slug: 'required-dependency',
    project_title: 'Required dependency',
    version_id: 'dependency-version',
    version_number: '1.0.0',
    filename: 'required-dependency-1.0.0.jar',
  }],
  dependency_count: 1,
  optional_dependencies_not_installed: ['Optional map integration', 'Optional map integration'],
  backup_id: '1800000000200',
  server_was_running: true,
  restart_required: false,
  runtime_validation_performed: true,
  rollback_on_failed_startup: true,
};

afterEach(() => vi.unstubAllGlobals());

describe('marketplace contracts', () => {
  it('parses search and project results without accepting incompatible versions', () => {
    const page = parseMarketplaceSearchPage(searchPage);
    const detail = parseMarketplaceProjectDetail(projectDetail);

    expect(page.hits[0]).toMatchObject({ projectId: 'P7dR8mSH', author: 'EngineHub', downloads: 42_000_000, iconUrl: expect.stringContaining('/marketplace/modrinth/image?') });
    expect(detail.versions.map((version) => version.versionType)).toEqual(['release', 'beta']);
    expect(detail.bodyFormat).toBe('plain_text');
    expect(marketplaceResponseMatchesServer(page.instanceId, page.compatibility, server)).toBe(true);

    const incompatible = structuredClone(projectDetail);
    incompatible.versions[0]!.game_versions = ['1.20.6'];
    expect(() => parseMarketplaceProjectDetail(incompatible)).toThrow(/outside the selected server compatibility/i);
  });

  it('keeps projects browseable when server-side metadata is unsupported or unknown', () => {
    const unsupported = structuredClone(projectDetail);
    unsupported.project.server_side = 'unsupported';
    expect(parseMarketplaceProjectDetail(unsupported).project.serverSide).toBe('unsupported');
    const unknown = structuredClone(projectDetail);
    unknown.project.server_side = 'unknown';
    expect(parseMarketplaceProjectDetail(unknown).project.serverSide).toBe('unknown');
  });

  it('rejects unsafe project links and unsafe installed filenames', () => {
    expect(() => parseMarketplaceSearchPage({
      ...searchPage,
      hits: [{ ...searchPage.hits[0], web_url: 'https://example.com/plugin/fabric-api' }],
    })).toThrow(/Modrinth URL/i);
    expect(() => parseMarketplaceInstallResult({
      ...installResult,
      installed_projects: [{ ...installResult.installed_projects[0], filename: '../server.properties.jar' }],
      dependency_count: 0,
    })).toThrow(/unsafe filename/i);
  });

  it('parses completed and failed installation jobs precisely', () => {
    const completed = parseMarketplaceInstallJob({
      id: jobId,
      kind: 'server_marketplace_install',
      status: 'complete',
      stage: 'Content installed',
      progress_percent: 100,
      created_at_unix_ms: 1_800_000_000_200,
      updated_at_unix_ms: 1_800_000_001_000,
      result: installResult,
      error: null,
    });
    const failed = parseMarketplaceInstallJob({
      id: jobId,
      kind: 'server_marketplace_install',
      status: 'failed',
      stage: 'Restoring backup',
      progress_percent: 82,
      created_at_unix_ms: 1_800_000_000_200,
      updated_at_unix_ms: 1_800_000_001_000,
      result: null,
      error: 'The new server failed its startup health check; the backup was restored.',
    });

    expect(completed.result).toMatchObject({ dependencyCount: 1, backupId: '1800000000200', restartRequired: false, runtimeValidationPerformed: true, rollbackOnFailedStartup: true });
    expect(completed.result?.optionalDependenciesNotInstalled).toHaveLength(2);
    expect(failed.error).toContain('backup was restored');
  });

  it('reports stopped installs as staged without invented startup rollback', () => {
    const stopped = parseMarketplaceInstallResult({
      ...installResult,
      server_was_running: false,
      restart_required: true,
      runtime_validation_performed: false,
      rollback_on_failed_startup: false,
    });
    expect(stopped).toMatchObject({
      serverWasRunning: false,
      restartRequired: true,
      runtimeValidationPerformed: false,
      rollbackOnFailedStartup: false,
    });
    expect(() => parseMarketplaceInstallResult({
      ...installResult,
      server_was_running: false,
      restart_required: true,
      runtime_validation_performed: false,
      rollback_on_failed_startup: true,
    })).toThrow(/inconsistent runtime validation guarantees/i);
  });

  it('only enables the native server families supported by the broker', () => {
    expect(marketplaceProfileForSoftware('Paper')?.contentKind).toBe('plugin');
    expect(marketplaceProfileForSoftware('Purpur')?.acceptedLoaders).toContain('paper');
    expect(marketplaceProfileForSoftware('Folia')?.acceptedLoaders).toEqual(['folia']);
    expect(marketplaceProfileForSoftware('Fabric')?.contentKind).toBe('mod');
    expect(marketplaceProfileForSoftware('Vanilla')).toBeNull();
    expect(marketplaceProfileForSoftware('NeoForge')).toBeNull();
    expect(marketplaceResponseMatchesServer(server.id, { ...parseMarketplaceSearchPage(searchPage).compatibility, minecraftVersion: '1.20.6' }, server)).toBe(false);
  });

  it('uses the exact search, detail, install, and job contracts', async () => {
    const responses = [
      searchPage,
      projectDetail,
      { job_id: jobId, reused: false },
      {
        id: jobId,
        kind: 'server_marketplace_install',
        status: 'complete',
        stage: 'Content installed',
        progress_percent: 100,
        created_at_unix_ms: 1_800_000_000_200,
        updated_at_unix_ms: 1_800_000_001_000,
        result: installResult,
        error: null,
      },
    ];
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(new Response(JSON.stringify(responses.shift()), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    })));
    vi.stubGlobal('fetch', fetchMock);

    await searchMarketplace(server.id, ' world edit ', 0, 20, csrf);
    await getMarketplaceProject(server.id, 'P7dR8mSH', csrf);
    await installMarketplaceProject(server.id, 'P7dR8mSH', 'paper-release-2131', csrf);
    await getMarketplaceInstallJob(jobId, csrf);

    expect(fetchMock.mock.calls.map((call) => call[0])).toEqual([
      `/api/v1/servers/${server.id}/marketplace/search?query=world+edit&offset=0&limit=20`,
      `/api/v1/servers/${server.id}/marketplace/projects/P7dR8mSH`,
      `/api/v1/servers/${server.id}/marketplace/install`,
      `/api/v1/jobs/${jobId}`,
    ]);
    const post = fetchMock.mock.calls[2]?.[1] as RequestInit;
    expect(post.method).toBe('POST');
    expect(post.headers).toMatchObject({ 'Content-Type': 'application/json', 'X-Helix-CSRF': csrf });
    expect(JSON.parse(String(post.body))).toEqual({ project_id: 'P7dR8mSH', version_id: 'paper-release-2131' });
  });
});
