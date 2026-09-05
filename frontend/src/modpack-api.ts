import { ApiError, expectArray, expectNumber, expectRecord, expectString, requestJson } from './api';

export type ModpackCompatibilityStatus = 'fabric_candidate' | 'forge_candidate' | 'neoforge_candidate' | 'quilt_candidate' | 'unverified' | 'incompatible';
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
  provider: ModpackProvider;
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
  provider: ModpackProvider;
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
  loaders: string[];
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
  provider: ModpackProvider;
  projectTitle: string;
  versionNumber: string;
  minecraftVersion: string;
  loader: string;
  loaderVersion: string;
  installedServerFiles: number;
  excludedServerOptionalFiles: number;
  excludedClientOnlyFiles: number;
  excludedNonJarFiles: number;
  excludedLaunchFiles: number;
  serverPackUsed: boolean;
  serverPackFilename: string | null;
  serverSafeSubset: boolean;
  fullPackParity: boolean;
  catalogIntegrityVerified: boolean;
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

function optionalBoolean(record: Record<string, unknown>, key: string, context: string): boolean | null {
  const value = record[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'boolean') throw new ApiError(`${context} returned an invalid ${key}.`);
  return value;
}

function modpackProvider(record: Record<string, unknown>, context: string): ModpackProvider {
  const value = expectString(record, 'provider', context);
  if (value !== 'modrinth' && value !== 'curseforge') throw new ApiError(`${context} returned an invalid provider.`);
  return value;
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
  const parsed = new URL(value, 'http://helix.invalid');
  const keys = Array.from(parsed.searchParams.keys());
  const path = parsed.searchParams.get('path');
  const origin = parsed.pathname === '/api/v1/marketplace/modrinth/image'
    ? 'modrinth'
    : parsed.pathname === '/api/v1/marketplace/curseforge/image'
      ? 'curseforge'
      : null;
  const prefix = origin === 'modrinth' ? '/data/' : origin === 'curseforge' ? '/avatars/' : null;
  if (
    origin === null
    || prefix === null
    || !value.startsWith(`/api/v1/marketplace/${origin}/image?`)
    || parsed.origin !== 'http://helix.invalid'
    || keys.length !== 1
    || keys[0] !== 'path'
    || path === null
    || path.length > 512
    || !path.startsWith(prefix)
    || path.split('/').some((segment) => segment === '.' || segment === '..')
    || !/^\/[A-Za-z0-9._/-]+$/u.test(path)
    || !/\.(?:png|jpe?g|webp|gif)$/iu.test(path)
  ) throw new ApiError(`${context} returned an unsafe ${key}.`);
  return value;
}

function parseSearchResult(value: unknown): ModpackSearchResult {
  const item = expectRecord(value, 'Modpack search result');
  const compatibilityStatus = expectString(item, 'compatibility_status', 'Modpack search result');
  if (compatibilityStatus !== 'fabric_candidate' && compatibilityStatus !== 'forge_candidate' && compatibilityStatus !== 'neoforge_candidate' && compatibilityStatus !== 'quilt_candidate' && compatibilityStatus !== 'unverified' && compatibilityStatus !== 'incompatible') {
    throw new ApiError('Modpack search returned an invalid compatibility state.');
  }
  return {
    projectId: expectString(item, 'project_id', 'Modpack search result'),
    slug: expectString(item, 'slug', 'Modpack search result'),
    title: expectString(item, 'title', 'Modpack search result'),
    description: optionalString(item, 'description', 'Modpack search result'),
    author: optionalString(item, 'author', 'Modpack search result'),
    downloads: expectNumber(item, 'downloads', 'Modpack search result', { integer: true, minimum: 0 }),
    follows: expectNumber(item, 'follows', 'Modpack search result', { integer: true, minimum: 0 }),
    latestVersion: optionalString(item, 'latest_version', 'Modpack search result'),
    minecraftVersions: stringList(item, 'minecraft_versions', 'Modpack search result'),
    loaders: stringList(item, 'loaders', 'Modpack search result'),
    serverSide: expectString(item, 'server_side', 'Modpack search result'),
    compatibilityStatus,
    compatibilityReason: expectString(item, 'compatibility_reason', 'Modpack search result'),
    requiresVersionCheck: boolean(item, 'requires_version_check', 'Modpack search result'),
    webUrl: packWebUrl(item, 'web_url', 'Modpack search result'),
    iconUrl: marketplaceImageUrl(item, 'icon_url', 'Modpack search result'),
  };
}

export function parseModpackSearchPage(value: unknown): ModpackSearchPage {
  const root = expectRecord(value, 'Modpack search');
  const provider = modpackProvider(root, 'Modpack search');
  const context = provider === 'curseforge' ? 'CurseForge modpack search' : 'Modrinth modpack search';
  if (expectNumber(root, 'schema_version', context, { integer: true }) !== 1) throw new ApiError(`${provider === 'curseforge' ? 'CurseForge' : 'Modrinth'} search returned an unsupported schema.`);
  if (typeof root.query !== 'string' || root.query.length > 120) throw new ApiError(`${provider === 'curseforge' ? 'CurseForge' : 'Modrinth'} search returned an invalid query.`);
  return {
    provider,
    query: root.query,
    offset: expectNumber(root, 'offset', context, { integer: true, minimum: 0 }),
    limit: expectNumber(root, 'limit', context, { integer: true, minimum: 1, maximum: 50 }),
    totalHits: expectNumber(root, 'total_hits', context, { integer: true, minimum: 0 }),
    results: expectArray(root, 'results', context, 50).map(parseSearchResult),
  };
}

function parseModpackVersion(value: unknown, provider: ModpackProvider): ModpackVersion {
  const catalog = provider === 'curseforge' ? 'CurseForge' : 'Modrinth';
  const context = `${catalog} modpack version`;
  const fileContext = `${catalog} pack file`;
  const item = expectRecord(value, context);
  const fileValue = item.mrpack_file;
  let mrpackFile: ModpackVersion['mrpackFile'] = null;
  if (fileValue !== null && fileValue !== undefined) {
    const file = expectRecord(fileValue, fileContext);
    mrpackFile = {
      filename: expectString(file, 'filename', fileContext),
      size: expectNumber(file, 'size', fileContext, { integer: true, minimum: 1 }),
      modrinthDeclaredSha512Available: boolean(file, 'modrinth_declared_sha512_available', fileContext),
    };
  }
  const installable = boolean(item, 'installable', context);
  if (installable && mrpackFile === null) throw new ApiError(`An installable ${catalog} version omitted its pack file.`);
  return {
    id: expectString(item, 'id', context),
    name: expectString(item, 'name', context),
    versionNumber: expectString(item, 'version_number', context),
    versionType: expectString(item, 'version_type', context),
    status: optionalString(item, 'status', context),
    datePublished: optionalString(item, 'date_published', context),
    downloads: expectNumber(item, 'downloads', context, { integer: true, minimum: 0 }),
    gameVersions: stringList(item, 'game_versions', context),
    loaders: stringList(item, 'loaders', context),
    installable,
    compatibilityReason: expectString(item, 'compatibility_reason', context),
    mrpackFile,
  };
}

export function parseModpackProjectDetail(value: unknown): ModpackProjectDetail {
  const root = expectRecord(value, 'Modpack project');
  const provider = modpackProvider(root, 'Modpack project');
  const catalog = provider === 'curseforge' ? 'CurseForge' : 'Modrinth';
  const context = `${catalog} modpack project`;
  if (expectNumber(root, 'schema_version', context, { integer: true }) !== 1) throw new ApiError(`${catalog} project returned an unsupported schema.`);
  const project = expectRecord(root.project, context);
  return {
    provider,
    project: {
      id: expectString(project, 'id', context),
      slug: expectString(project, 'slug', context),
      title: expectString(project, 'title', context),
      description: optionalString(project, 'description', context),
      body: optionalString(project, 'body', context),
      author: optionalString(project, 'author', context),
      downloads: expectNumber(project, 'downloads', context, { integer: true, minimum: 0 }),
      followers: expectNumber(project, 'followers', context, { integer: true, minimum: 0 }),
      serverSide: expectString(project, 'server_side', context),
      clientSide: optionalString(project, 'client_side', context),
      loaders: stringList(project, 'loaders', context, 32),
      webUrl: packWebUrl(project, 'web_url', context),
      iconUrl: marketplaceImageUrl(project, 'icon_url', context),
    },
    versions: expectArray(root, 'versions', context, 200).map((version) => parseModpackVersion(version, provider)),
    compatibleVersionCount: expectNumber(root, 'compatible_version_count', context, { integer: true, minimum: 0 }),
    versionResultsTruncated: boolean(root, 'version_results_truncated', context),
  };
}

export function parseMinecraftModpackCreateResult(value: unknown): MinecraftModpackCreateResult {
  const root = expectRecord(value, 'modpack creation result');
  const modpack = expectRecord(root.modpack, 'modpack creation result');
  const providerValue = optionalString(modpack, 'provider', 'modpack creation result') ?? 'modrinth';
  if (providerValue !== 'modrinth' && providerValue !== 'curseforge') throw new ApiError('Modpack creation returned an invalid provider.');
  const provider: ModpackProvider = providerValue;
  if (provider === 'curseforge') {
    const serverPackUsed = optionalBoolean(modpack, 'server_pack_used', 'modpack creation result') ?? false;
    return {
      provider,
      projectTitle: optionalString(modpack, 'project_title', 'modpack creation result') ?? 'CurseForge pack',
      versionNumber: optionalString(modpack, 'version_number', 'modpack creation result') ?? optionalString(modpack, 'version_id', 'modpack creation result') ?? expectString(modpack, 'source_filename', 'modpack creation result'),
      minecraftVersion: expectString(modpack, 'minecraft_version', 'modpack creation result'),
      loader: optionalString(modpack, 'loader', 'modpack creation result') ?? 'unknown',
      loaderVersion: optionalString(modpack, 'loader_version', 'modpack creation result') ?? 'unknown',
      installedServerFiles: expectNumber(modpack, 'installed_server_files', 'modpack creation result', { integer: true, minimum: 0 }),
      excludedServerOptionalFiles: 0,
      excludedClientOnlyFiles: 0,
      excludedNonJarFiles: modpack.excluded_non_jar_files === undefined ? 0 : expectNumber(modpack, 'excluded_non_jar_files', 'modpack creation result', { integer: true, minimum: 0 }),
      excludedLaunchFiles: modpack.excluded_launch_files === undefined ? 0 : expectNumber(modpack, 'excluded_launch_files', 'modpack creation result', { integer: true, minimum: 0 }),
      serverPackUsed,
      serverPackFilename: optionalString(modpack, 'server_pack_filename', 'modpack creation result'),
      serverSafeSubset: !serverPackUsed,
      fullPackParity: false,
      catalogIntegrityVerified: true,
    };
  }
  const parsed = {
    provider,
    projectTitle: expectString(modpack, 'project_title', 'modpack creation result'),
    versionNumber: expectString(modpack, 'version_number', 'modpack creation result'),
    minecraftVersion: expectString(modpack, 'minecraft_version', 'modpack creation result'),
    loader: optionalString(modpack, 'loader', 'modpack creation result') ?? 'fabric',
    loaderVersion: optionalString(modpack, 'loader_version', 'modpack creation result') ?? expectString(modpack, 'fabric_loader_version', 'modpack creation result'),
    installedServerFiles: expectNumber(modpack, 'installed_server_files', 'modpack creation result', { integer: true, minimum: 0 }),
    excludedServerOptionalFiles: expectNumber(modpack, 'excluded_server_optional_files', 'modpack creation result', { integer: true, minimum: 0 }),
    excludedClientOnlyFiles: expectNumber(modpack, 'excluded_client_only_files', 'modpack creation result', { integer: true, minimum: 0 }),
    excludedNonJarFiles: 0,
    excludedLaunchFiles: 0,
    serverPackUsed: false,
    serverPackFilename: null,
    serverSafeSubset: boolean(modpack, 'server_safe_subset', 'modpack creation result'),
    fullPackParity: boolean(modpack, 'full_pack_parity', 'modpack creation result'),
    catalogIntegrityVerified: boolean(root, 'modrinth_declared_sha512_verified', 'modpack creation result'),
  };
  if (!parsed.serverSafeSubset || parsed.fullPackParity || !parsed.catalogIntegrityVerified) throw new ApiError('Modpack creation returned inconsistent safety verification.');
  return parsed;
}

export function searchModpacks(query: string, offset: number, limit: number, csrfToken: string, signal?: AbortSignal, provider: ModpackProvider = 'modrinth'): Promise<ModpackSearchPage> {
  const params = new URLSearchParams({ query, offset: String(offset), limit: String(limit), provider });
  return requestJson(`/api/v1/servers/minecraft/modpacks/search?${params}`, parseModpackSearchPage, { csrfToken, signal, timeoutMs: 40_000 });
}

export function getModpackProject(projectId: string, csrfToken: string, signal?: AbortSignal, provider: ModpackProvider = 'modrinth'): Promise<ModpackProjectDetail> {
  const params = new URLSearchParams({ provider });
  return requestJson(`/api/v1/servers/minecraft/modpacks/projects/${encodeURIComponent(projectId)}?${params}`, parseModpackProjectDetail, { csrfToken, signal, timeoutMs: 40_000 });
}

export function createMinecraftModpack(input: MinecraftModpackCreateInput, csrfToken: string): Promise<{ jobId: string }> {
  return requestJson('/api/v1/servers/minecraft/modpacks', (value) => {
    const root = expectRecord(value, 'modpack creation job');
    return { jobId: expectString(root, 'job_id', 'modpack creation job') };
  }, { method: 'POST', body: input, csrfToken, timeoutMs: 20_000 });
}
