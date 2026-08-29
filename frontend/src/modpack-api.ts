import { ApiError, expectArray, expectNumber, expectRecord, expectString, requestJson } from './api';

export type ModpackCompatibilityStatus = 'fabric_candidate' | 'forge_candidate' | 'neoforge_candidate' | 'quilt_candidate' | 'incompatible';
export type ModpackProvider = 'modrinth' | 'curseforge';

export interface ModpackSearchResult {
  projectId: string;
  slug: string;
  title: string;
  description: string | null;
  author: string | null;
  downloads: number;
  follows: number;
  latestVersion: string | null;
  minecraftVersions: string[];
  loaders: string[];
  serverSide: string;
  compatibilityStatus: ModpackCompatibilityStatus;
  compatibilityReason: string;
  requiresVersionCheck: boolean;
  webUrl: string;
  iconUrl: string | null;
}

export interface ModpackSearchPage {
  query: string;
  offset: number;
  limit: number;
  totalHits: number;
  results: ModpackSearchResult[];
}

export interface ModpackVersion {
  id: string;
  name: string;
  versionNumber: string;
  versionType: string;
  status: string | null;
  datePublished: string | null;
  downloads: number;
  gameVersions: string[];
  loaders: string[];
  installable: boolean;
  compatibilityReason: string;
  mrpackFile: null | {
    filename: string;
    size: number;
    modrinthDeclaredSha512Available: boolean;
  };
}

export interface ModpackProject {
  id: string;
  slug: string;
  title: string;
  description: string | null;
  body: string | null;
  author: string | null;
  downloads: number;
  followers: number;
  serverSide: string;
  clientSide: string | null;
  loaders: string[];
  webUrl: string;
  iconUrl: string | null;
}

export interface ModpackProjectDetail {
  project: ModpackProject;
  versions: ModpackVersion[];
  compatibleVersionCount: number;
  versionResultsTruncated: boolean;
}

export interface ModpackSelection {
  projectId: string;
  projectSlug: string;
  projectTitle: string;
  projectWebUrl: string;
  versionId: string;
  versionName: string;
  versionNumber: string;
  minecraftVersions: string[];
  filename: string;
  fileSize: number;
  provider: ModpackProvider;
}

export interface MinecraftModpackCreateInput {
  name: string;
  memory_mb: number;
  cpu_millis?: number;
  max_players: number;
  game_port?: number;
  network_exposure: 'private' | 'public';
  start_on_boot: boolean;
  eula_accepted: boolean;
  project_id: string;
  version_id: string;
  provider?: ModpackProvider;
}

export interface MinecraftModpackCreateResult {
  projectTitle: string;
  versionNumber: string;
  minecraftVersion: string;
  fabricLoaderVersion: string;
  installedServerFiles: number;
  excludedServerOptionalFiles: number;
  excludedClientOnlyFiles: number;
  serverSafeSubset: boolean;
  fullPackParity: boolean;
  modrinthDeclaredSha512Verified: boolean;
}

function optionalString(record: Record<string, unknown>, key: string, context: string): string | null {
  const value = record[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'string' || value.length > 131_072) throw new ApiError(`${context} returned an invalid ${key}.`);
  return value;
}

function boolean(record: Record<string, unknown>, key: string, context: string): boolean {
  if (typeof record[key] !== 'boolean') throw new ApiError(`${context} returned an invalid ${key}.`);
  return record[key];
}

function stringList(record: Record<string, unknown>, key: string, context: string, maximum = 128): string[] {
  return expectArray(record, key, context, maximum).map((value) => {
    if (typeof value !== 'string' || value.length === 0 || value.length > 128) throw new ApiError(`${context} returned an invalid ${key}.`);
    return value;
  });
}

function packWebUrl(record: Record<string, unknown>, key: string, context: string): string {
  const value = expectString(record, key, context);
  if (
    /^https:\/\/modrinth\.com\/modpack\/[A-Za-z0-9_-]+$/u.test(value)
    || /^https:\/\/www\.curseforge\.com\/minecraft\/modpacks\/[A-Za-z0-9_-]+$/u.test(value)
  ) {
    return value;
  }
  throw new ApiError(`${context} returned an invalid pack URL.`);
}

function marketplaceImageUrl(record: Record<string, unknown>, key: string, context: string): string | null {
  const value = optionalString(record, key, context);
  if (value === null) return null;
  if (/^https:\/\/(?:(?:www|[a-z0-9-]+)\.)?(?:curseforge|forgecdn)\.com\//iu.test(value)) return value;
  const parsed = new URL(value, 'http://helix.invalid');
  const keys = Array.from(parsed.searchParams.keys());
  const path = parsed.searchParams.get('path');
  if (
    !value.startsWith('/api/v1/marketplace/modrinth/image?')
    || parsed.origin !== 'http://helix.invalid'
    || parsed.pathname !== '/api/v1/marketplace/modrinth/image'
    || keys.length !== 1
    || keys[0] !== 'path'
    || path === null
    || path.length > 512
    || !path.startsWith('/data/')
    || path.split('/').some((segment) => segment === '.' || segment === '..')
    || !/^\/[A-Za-z0-9._/-]+$/u.test(path)
    || !/\.(?:png|jpe?g|webp|gif)$/iu.test(path)
  ) throw new ApiError(`${context} returned an unsafe ${key}.`);
  return value;
}

function parseSearchResult(value: unknown): ModpackSearchResult {
  const item = expectRecord(value, 'Modrinth search result');
  const compatibilityStatus = expectString(item, 'compatibility_status', 'Modrinth search result');
  if (compatibilityStatus !== 'fabric_candidate' && compatibilityStatus !== 'forge_candidate' && compatibilityStatus !== 'neoforge_candidate' && compatibilityStatus !== 'quilt_candidate' && compatibilityStatus !== 'incompatible') {
    throw new ApiError('Modpack search returned an invalid compatibility state.');
  }
  return {
    projectId: expectString(item, 'project_id', 'Modrinth search result'),
    slug: expectString(item, 'slug', 'Modrinth search result'),
    title: expectString(item, 'title', 'Modrinth search result'),
    description: optionalString(item, 'description', 'Modrinth search result'),
    author: optionalString(item, 'author', 'Modrinth search result'),
    downloads: expectNumber(item, 'downloads', 'Modrinth search result', { integer: true, minimum: 0 }),
    follows: expectNumber(item, 'follows', 'Modrinth search result', { integer: true, minimum: 0 }),
    latestVersion: optionalString(item, 'latest_version', 'Modrinth search result'),
    minecraftVersions: stringList(item, 'minecraft_versions', 'Modrinth search result'),
    loaders: stringList(item, 'loaders', 'Modrinth search result'),
    serverSide: expectString(item, 'server_side', 'Modrinth search result'),
    compatibilityStatus,
    compatibilityReason: expectString(item, 'compatibility_reason', 'Modrinth search result'),
    requiresVersionCheck: boolean(item, 'requires_version_check', 'Modrinth search result'),
    webUrl: packWebUrl(item, 'web_url', 'Modrinth search result'),
    iconUrl: marketplaceImageUrl(item, 'icon_url', 'Modrinth search result'),
  };
}

export function parseModpackSearchPage(value: unknown): ModpackSearchPage {
  const root = expectRecord(value, 'Modrinth modpack search');
  if (expectNumber(root, 'schema_version', 'Modrinth modpack search', { integer: true }) !== 1) throw new ApiError('Modrinth search returned an unsupported schema.');
  if (typeof root.query !== 'string' || root.query.length > 120) throw new ApiError('Modrinth search returned an invalid query.');
  return {
    query: root.query,
    offset: expectNumber(root, 'offset', 'Modrinth modpack search', { integer: true, minimum: 0 }),
    limit: expectNumber(root, 'limit', 'Modrinth modpack search', { integer: true, minimum: 1, maximum: 50 }),
    totalHits: expectNumber(root, 'total_hits', 'Modrinth modpack search', { integer: true, minimum: 0 }),
    results: expectArray(root, 'results', 'Modrinth modpack search', 50).map(parseSearchResult),
  };
}

function parseModpackVersion(value: unknown): ModpackVersion {
  const item = expectRecord(value, 'Modrinth modpack version');
  const fileValue = item.mrpack_file;
  let mrpackFile: ModpackVersion['mrpackFile'] = null;
  if (fileValue !== null && fileValue !== undefined) {
    const file = expectRecord(fileValue, 'Modrinth .mrpack file');
    mrpackFile = {
      filename: expectString(file, 'filename', 'Modrinth .mrpack file'),
      size: expectNumber(file, 'size', 'Modrinth .mrpack file', { integer: true, minimum: 1 }),
      modrinthDeclaredSha512Available: boolean(file, 'modrinth_declared_sha512_available', 'Modrinth .mrpack file'),
    };
  }
  const installable = boolean(item, 'installable', 'Modrinth modpack version');
  if (installable && mrpackFile === null) throw new ApiError('An installable Modrinth version omitted its .mrpack file.');
  return {
    id: expectString(item, 'id', 'Modrinth modpack version'),
    name: expectString(item, 'name', 'Modrinth modpack version'),
    versionNumber: expectString(item, 'version_number', 'Modrinth modpack version'),
    versionType: expectString(item, 'version_type', 'Modrinth modpack version'),
    status: optionalString(item, 'status', 'Modrinth modpack version'),
    datePublished: optionalString(item, 'date_published', 'Modrinth modpack version'),
    downloads: expectNumber(item, 'downloads', 'Modrinth modpack version', { integer: true, minimum: 0 }),
    gameVersions: stringList(item, 'game_versions', 'Modrinth modpack version'),
    loaders: stringList(item, 'loaders', 'Modrinth modpack version'),
    installable,
    compatibilityReason: expectString(item, 'compatibility_reason', 'Modrinth modpack version'),
    mrpackFile,
  };
}

export function parseModpackProjectDetail(value: unknown): ModpackProjectDetail {
  const root = expectRecord(value, 'Modrinth modpack project');
  if (expectNumber(root, 'schema_version', 'Modrinth modpack project', { integer: true }) !== 1) throw new ApiError('Modrinth project returned an unsupported schema.');
  const project = expectRecord(root.project, 'Modrinth modpack project');
  return {
    project: {
      id: expectString(project, 'id', 'Modrinth modpack project'),
      slug: expectString(project, 'slug', 'Modrinth modpack project'),
      title: expectString(project, 'title', 'Modrinth modpack project'),
      description: optionalString(project, 'description', 'Modrinth modpack project'),
      body: optionalString(project, 'body', 'Modrinth modpack project'),
      author: optionalString(project, 'author', 'Modrinth modpack project'),
      downloads: expectNumber(project, 'downloads', 'Modrinth modpack project', { integer: true, minimum: 0 }),
      followers: expectNumber(project, 'followers', 'Modrinth modpack project', { integer: true, minimum: 0 }),
      serverSide: expectString(project, 'server_side', 'Modrinth modpack project'),
      clientSide: optionalString(project, 'client_side', 'Modrinth modpack project'),
      loaders: stringList(project, 'loaders', 'Modrinth modpack project', 32),
      webUrl: packWebUrl(project, 'web_url', 'Modrinth modpack project'),
      iconUrl: marketplaceImageUrl(project, 'icon_url', 'Modrinth modpack project'),
    },
    versions: expectArray(root, 'versions', 'Modrinth modpack project', 200).map(parseModpackVersion),
    compatibleVersionCount: expectNumber(root, 'compatible_version_count', 'Modrinth modpack project', { integer: true, minimum: 0 }),
    versionResultsTruncated: boolean(root, 'version_results_truncated', 'Modrinth modpack project'),
  };
}

export function parseMinecraftModpackCreateResult(value: unknown): MinecraftModpackCreateResult {
  const root = expectRecord(value, 'modpack creation result');
  const modpack = expectRecord(root.modpack, 'modpack creation result');
  const provider = optionalString(modpack, 'provider', 'modpack creation result');
  if (provider === 'curseforge') {
    return {
      projectTitle: optionalString(modpack, 'project_title', 'modpack creation result') ?? 'CurseForge pack',
      versionNumber: optionalString(modpack, 'version_id', 'modpack creation result') ?? expectString(modpack, 'source_filename', 'modpack creation result'),
      minecraftVersion: expectString(modpack, 'minecraft_version', 'modpack creation result'),
      fabricLoaderVersion: optionalString(modpack, 'loader', 'modpack creation result') ?? 'unknown',
      installedServerFiles: expectNumber(modpack, 'installed_server_files', 'modpack creation result', { integer: true, minimum: 0 }),
      excludedServerOptionalFiles: 0,
      excludedClientOnlyFiles: 0,
      serverSafeSubset: true,
      fullPackParity: false,
      modrinthDeclaredSha512Verified: false,
    };
  }
  const parsed = {
    projectTitle: expectString(modpack, 'project_title', 'modpack creation result'),
    versionNumber: expectString(modpack, 'version_number', 'modpack creation result'),
    minecraftVersion: expectString(modpack, 'minecraft_version', 'modpack creation result'),
    fabricLoaderVersion: expectString(modpack, 'fabric_loader_version', 'modpack creation result'),
    installedServerFiles: expectNumber(modpack, 'installed_server_files', 'modpack creation result', { integer: true, minimum: 0 }),
    excludedServerOptionalFiles: expectNumber(modpack, 'excluded_server_optional_files', 'modpack creation result', { integer: true, minimum: 0 }),
    excludedClientOnlyFiles: expectNumber(modpack, 'excluded_client_only_files', 'modpack creation result', { integer: true, minimum: 0 }),
    serverSafeSubset: boolean(modpack, 'server_safe_subset', 'modpack creation result'),
    fullPackParity: boolean(modpack, 'full_pack_parity', 'modpack creation result'),
    modrinthDeclaredSha512Verified: boolean(root, 'modrinth_declared_sha512_verified', 'modpack creation result'),
  };
  if (!parsed.serverSafeSubset || parsed.fullPackParity || !parsed.modrinthDeclaredSha512Verified) throw new ApiError('Modpack creation returned inconsistent safety verification.');
  return parsed;
}

export function searchModpacks(query: string, offset: number, limit: number, csrfToken: string, signal?: AbortSignal, provider: ModpackProvider = 'modrinth'): Promise<ModpackSearchPage> {
  const params = new URLSearchParams({ query, offset: String(offset), limit: String(limit), provider });
  return requestJson(`/api/v1/servers/minecraft/modpacks/search?${params}`, parseModpackSearchPage, { csrfToken, signal, timeoutMs: 25_000 });
}

export function getModpackProject(projectId: string, csrfToken: string, signal?: AbortSignal): Promise<ModpackProjectDetail> {
  return requestJson(`/api/v1/servers/minecraft/modpacks/projects/${encodeURIComponent(projectId)}`, parseModpackProjectDetail, { csrfToken, signal, timeoutMs: 25_000 });
}

export function createMinecraftModpack(input: MinecraftModpackCreateInput, csrfToken: string): Promise<{ jobId: string }> {
  return requestJson('/api/v1/servers/minecraft/modpacks', (value) => {
    const root = expectRecord(value, 'modpack creation job');
    return { jobId: expectString(root, 'job_id', 'modpack creation job') };
  }, { method: 'POST', body: input, csrfToken, timeoutMs: 20_000 });
}
