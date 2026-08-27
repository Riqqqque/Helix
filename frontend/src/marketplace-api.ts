import { ApiError, expectArray, expectNumber, expectRecord, expectString, requestJson } from './api';

export type MarketplaceContentKind = 'plugin' | 'mod';
export type MarketplaceVersionChannel = 'release' | 'beta' | 'alpha';

export interface MarketplaceServerContext {
  id: string;
  name: string;
  software: string;
  minecraftVersion: string;
  status: 'online' | 'starting' | 'stopped';
}

export interface MarketplaceCompatibility {
  minecraftVersion: string;
  serverSoftware: string;
  contentKind: MarketplaceContentKind;
  acceptedLoaders: string[];
  installDirectory: 'plugins' | 'mods';
}

export interface MarketplaceSearchHit {
  projectId: string;
  slug: string;
  title: string;
  description: string | null;
  author: string | null;
  projectType: string;
  serverSide: string | null;
  downloads: number;
  follows: number;
  latestVersion: string | null;
  dateModified: string | null;
  webUrl: string;
  iconUrl: string | null;
}

export interface MarketplaceSearchPage {
  instanceId: string;
  compatibility: MarketplaceCompatibility;
  totalHits: number;
  offset: number;
  limit: number;
  hits: MarketplaceSearchHit[];
  collectedAtUnixMs: number;
}

export interface MarketplaceVersion {
  id: string;
  name: string;
  versionNumber: string;
  versionType: MarketplaceVersionChannel;
  datePublished: string | null;
  downloads: number;
  gameVersions: string[];
  loaders: string[];
  hasPrimaryFile: boolean;
}

export interface MarketplaceProjectDetail {
  instanceId: string;
  compatibility: MarketplaceCompatibility;
  project: {
    id: string;
    slug: string;
    title: string;
    description: string | null;
    body: string | null;
    projectType: string;
    contentKind: MarketplaceContentKind;
    serverSide: 'required' | 'optional';
    downloads: number;
    followers: number;
    license: string | null;
    sourceUrl: string | null;
    issuesUrl: string | null;
    wikiUrl: string | null;
    webUrl: string;
    iconUrl: string | null;
  };
  versions: MarketplaceVersion[];
  versionCountReturned: number;
  versionResultsTruncated: boolean;
  bodyFormat: 'plain_text';
  collectedAtUnixMs: number;
}

export interface MarketplaceInstallDispatch {
  jobId: string;
  reused: boolean;
}

export interface MarketplaceInstalledProject {
  projectId: string;
  projectSlug: string;
  projectTitle: string;
  versionId: string;
  versionNumber: string;
  filename: string;
}

export interface MarketplaceInstallResult {
  instanceId: string;
  projectId: string;
  projectSlug: string;
  projectTitle: string;
  versionId: string;
  versionNumber: string;
  installedProjects: MarketplaceInstalledProject[];
  dependencyCount: number;
  optionalDependenciesNotInstalled: string[];
  backupId: string;
  serverWasRunning: boolean;
  restartRequired: boolean;
  runtimeValidationPerformed: boolean;
  rollbackOnFailedStartup: boolean;
}

export interface MarketplaceInstallJob {
  id: string;
  kind: 'server_marketplace_install';
  status: 'queued' | 'running' | 'complete' | 'failed';
  stage: string;
  progressPercent: number;
  createdAtUnixMs: number;
  updatedAtUnixMs: number;
  result: MarketplaceInstallResult | null;
  error: string | null;
}

interface MarketplaceProfile {
  contentKind: MarketplaceContentKind;
  acceptedLoaders: readonly string[];
  installDirectory: 'plugins' | 'mods';
}

const profiles: Record<string, MarketplaceProfile> = {
  paper: { contentKind: 'plugin', acceptedLoaders: ['paper', 'spigot', 'bukkit'], installDirectory: 'plugins' },
  purpur: { contentKind: 'plugin', acceptedLoaders: ['purpur', 'paper', 'spigot', 'bukkit'], installDirectory: 'plugins' },
  folia: { contentKind: 'plugin', acceptedLoaders: ['folia'], installDirectory: 'plugins' },
  fabric: { contentKind: 'mod', acceptedLoaders: ['fabric'], installDirectory: 'mods' },
};

export function marketplaceProfileForSoftware(software: string): MarketplaceProfile | null {
  return profiles[software.trim().toLowerCase()] ?? null;
}

export function marketplaceResponseMatchesServer(
  instanceId: string,
  compatibility: MarketplaceCompatibility,
  server: MarketplaceServerContext,
): boolean {
  const profile = marketplaceProfileForSoftware(server.software);
  if (profile === null) return false;
  const accepted = new Set(compatibility.acceptedLoaders.map((loader) => loader.toLowerCase()));
  return instanceId === server.id
    && compatibility.minecraftVersion === server.minecraftVersion
    && compatibility.serverSoftware.trim().toLowerCase() === server.software.trim().toLowerCase()
    && compatibility.contentKind === profile.contentKind
    && compatibility.installDirectory === profile.installDirectory
    && accepted.size === profile.acceptedLoaders.length
    && profile.acceptedLoaders.every((loader) => accepted.has(loader));
}

function boolean(record: Record<string, unknown>, key: string, context: string): boolean {
  if (typeof record[key] !== 'boolean') throw new ApiError(`${context} returned an invalid ${key} value.`);
  return record[key];
}

function nullableText(record: Record<string, unknown>, key: string, context: string, maximumBytes: number): string | null {
  const value = record[key];
  if (value === null) return null;
  if (typeof value !== 'string' || new TextEncoder().encode(value).length > maximumBytes) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }
  return value;
}

function textList(record: Record<string, unknown>, key: string, context: string, maximum: number): string[] {
  return expectArray(record, key, context, maximum).map((value) => {
    if (typeof value !== 'string' || value.length === 0 || value.length > 128) {
      throw new ApiError(`${context} returned an invalid ${key} value.`);
    }
    return value;
  });
}

function nonnegative(record: Record<string, unknown>, key: string, context: string): number {
  return expectNumber(record, key, context, { integer: true, minimum: 0 });
}

function marketplaceId(record: Record<string, unknown>, key: string, context: string): string {
  const value = expectString(record, key, context);
  if (!/^[A-Za-z0-9_-]{1,64}$/u.test(value)) throw new ApiError(`${context} returned an invalid ${key} value.`);
  return value;
}

function uuid(record: Record<string, unknown>, key: string, context: string): string {
  const value = expectString(record, key, context);
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(value)) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }
  return value;
}

function literal<T extends string>(record: Record<string, unknown>, key: string, context: string, values: readonly T[]): T {
  const value = expectString(record, key, context);
  if (!values.includes(value as T)) throw new ApiError(`${context} returned an invalid ${key} value.`);
  return value as T;
}

function optionalHttpsUrl(record: Record<string, unknown>, key: string, context: string): string | null {
  const value = nullableText(record, key, context, 4_096);
  if (value === null) return null;
  try {
    const parsed = new URL(value);
    if (parsed.protocol !== 'https:' || parsed.username !== '' || parsed.password !== '') throw new Error();
  } catch {
    throw new ApiError(`${context} returned an unsafe ${key} value.`);
  }
  return value;
}

function modrinthUrl(record: Record<string, unknown>, key: string, context: string): string {
  const value = optionalHttpsUrl(record, key, context);
  if (value === null) throw new ApiError(`${context} returned a missing ${key} value.`);
  const parsed = new URL(value);
  if (parsed.hostname !== 'modrinth.com' || !/^\/(?:plugin|mod)\//u.test(parsed.pathname)) {
    throw new ApiError(`${context} returned an invalid Modrinth URL.`);
  }
  return value;
}

function marketplaceImageUrl(record: Record<string, unknown>, key: string, context: string): string | null {
  const value = nullableText(record, key, context, 1_024);
  if (value === null) return null;
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
  ) {
    throw new ApiError(`${context} returned an unsafe ${key} value.`);
  }
  return value;
}

function parseCompatibility(value: unknown): MarketplaceCompatibility {
  const root = expectRecord(value, 'marketplace compatibility');
  const contentKind = literal(root, 'content_kind', 'marketplace compatibility', ['plugin', 'mod'] as const);
  const installDirectory = literal(root, 'install_directory', 'marketplace compatibility', ['plugins', 'mods'] as const);
  if ((contentKind === 'plugin') !== (installDirectory === 'plugins')) {
    throw new ApiError('Marketplace compatibility returned an inconsistent install directory.');
  }
  return {
    minecraftVersion: expectString(root, 'minecraft_version', 'marketplace compatibility'),
    serverSoftware: expectString(root, 'server_software', 'marketplace compatibility'),
    contentKind,
    acceptedLoaders: textList(root, 'accepted_loaders', 'marketplace compatibility', 8),
    installDirectory,
  };
}

function parseSearchHit(value: unknown): MarketplaceSearchHit {
  const root = expectRecord(value, 'marketplace search hit');
  return {
    projectId: marketplaceId(root, 'project_id', 'marketplace search hit'),
    slug: expectString(root, 'slug', 'marketplace search hit'),
    title: expectString(root, 'title', 'marketplace search hit'),
    description: nullableText(root, 'description', 'marketplace search hit', 2_048),
    author: nullableText(root, 'author', 'marketplace search hit', 128),
    projectType: expectString(root, 'project_type', 'marketplace search hit'),
    serverSide: nullableText(root, 'server_side', 'marketplace search hit', 32),
    downloads: nonnegative(root, 'downloads', 'marketplace search hit'),
    follows: nonnegative(root, 'follows', 'marketplace search hit'),
    latestVersion: nullableText(root, 'latest_version', 'marketplace search hit', 128),
    dateModified: nullableText(root, 'date_modified', 'marketplace search hit', 64),
    webUrl: modrinthUrl(root, 'web_url', 'marketplace search hit'),
    iconUrl: marketplaceImageUrl(root, 'icon_url', 'marketplace search hit'),
  };
}

export function parseMarketplaceSearchPage(value: unknown): MarketplaceSearchPage {
  const root = expectRecord(value, 'marketplace search');
  if (nonnegative(root, 'schema_version', 'marketplace search') !== 1) throw new ApiError('Marketplace search returned an unsupported schema.');
  const limit = expectNumber(root, 'limit', 'marketplace search', { integer: true, minimum: 1, maximum: 50 });
  const hits = expectArray(root, 'hits', 'marketplace search', 50).map(parseSearchHit);
  if (hits.length > limit) throw new ApiError('Marketplace search returned more results than its page limit.');
  return {
    instanceId: expectString(root, 'instance_id', 'marketplace search'),
    compatibility: parseCompatibility(root.compatibility),
    totalHits: nonnegative(root, 'total_hits', 'marketplace search'),
    offset: expectNumber(root, 'offset', 'marketplace search', { integer: true, minimum: 0, maximum: 10_000 }),
    limit,
    hits,
    collectedAtUnixMs: nonnegative(root, 'collected_at_unix_ms', 'marketplace search'),
  };
}

function parseMarketplaceVersion(value: unknown): MarketplaceVersion {
  const root = expectRecord(value, 'marketplace version');
  return {
    id: marketplaceId(root, 'id', 'marketplace version'),
    name: expectString(root, 'name', 'marketplace version'),
    versionNumber: expectString(root, 'version_number', 'marketplace version'),
    versionType: literal(root, 'version_type', 'marketplace version', ['release', 'beta', 'alpha'] as const),
    datePublished: nullableText(root, 'date_published', 'marketplace version', 64),
    downloads: nonnegative(root, 'downloads', 'marketplace version'),
    gameVersions: textList(root, 'game_versions', 'marketplace version', 64),
    loaders: textList(root, 'loaders', 'marketplace version', 64),
    hasPrimaryFile: boolean(root, 'has_primary_file', 'marketplace version'),
  };
}

export function parseMarketplaceProjectDetail(value: unknown): MarketplaceProjectDetail {
  const root = expectRecord(value, 'marketplace project');
  if (nonnegative(root, 'schema_version', 'marketplace project') !== 1) throw new ApiError('Marketplace project returned an unsupported schema.');
  const compatibility = parseCompatibility(root.compatibility);
  const project = expectRecord(root.project, 'marketplace project details');
  const projectContentKind = literal(project, 'content_kind', 'marketplace project details', ['plugin', 'mod'] as const);
  if (projectContentKind !== compatibility.contentKind) throw new ApiError('Marketplace project returned inconsistent content types.');
  const versions = expectArray(root, 'versions', 'marketplace project', 100).map(parseMarketplaceVersion);
  if (versions.some((version) => !version.gameVersions.includes(compatibility.minecraftVersion) || !version.loaders.some((loader) => compatibility.acceptedLoaders.includes(loader)))) {
    throw new ApiError('Marketplace project returned a version outside the selected server compatibility.');
  }
  const versionCountReturned = expectNumber(root, 'version_count_returned', 'marketplace project', { integer: true, minimum: 0, maximum: 100 });
  if (versionCountReturned !== versions.length) throw new ApiError('Marketplace project returned an inconsistent version count.');
  return {
    instanceId: expectString(root, 'instance_id', 'marketplace project'),
    compatibility,
    project: {
      id: marketplaceId(project, 'id', 'marketplace project details'),
      slug: expectString(project, 'slug', 'marketplace project details'),
      title: expectString(project, 'title', 'marketplace project details'),
      description: nullableText(project, 'description', 'marketplace project details', 2_048),
      body: nullableText(project, 'body', 'marketplace project details', 512 * 1_024),
      projectType: expectString(project, 'project_type', 'marketplace project details'),
      contentKind: projectContentKind,
      serverSide: literal(project, 'server_side', 'marketplace project details', ['required', 'optional'] as const),
      downloads: nonnegative(project, 'downloads', 'marketplace project details'),
      followers: nonnegative(project, 'followers', 'marketplace project details'),
      license: nullableText(project, 'license', 'marketplace project details', 128),
      sourceUrl: optionalHttpsUrl(project, 'source_url', 'marketplace project details'),
      issuesUrl: optionalHttpsUrl(project, 'issues_url', 'marketplace project details'),
      wikiUrl: optionalHttpsUrl(project, 'wiki_url', 'marketplace project details'),
      webUrl: modrinthUrl(project, 'web_url', 'marketplace project details'),
      iconUrl: marketplaceImageUrl(project, 'icon_url', 'marketplace project details'),
    },
    versions,
    versionCountReturned,
    versionResultsTruncated: boolean(root, 'version_results_truncated', 'marketplace project'),
    bodyFormat: literal(root, 'body_format', 'marketplace project', ['plain_text'] as const),
    collectedAtUnixMs: nonnegative(root, 'collected_at_unix_ms', 'marketplace project'),
  };
}

function parseInstalledProject(value: unknown): MarketplaceInstalledProject {
  const root = expectRecord(value, 'installed marketplace project');
  const filename = expectString(root, 'filename', 'installed marketplace project');
  const hasControlCharacter = Array.from(filename).some((character) => {
    const point = character.codePointAt(0) ?? 0;
    return point <= 31 || point === 127;
  });
  if (filename.length > 180 || filename.startsWith('.') || !filename.toLowerCase().endsWith('.jar') || filename.includes('/') || filename.includes('\\') || hasControlCharacter) {
    throw new ApiError('Installed marketplace project returned an unsafe filename.');
  }
  return {
    projectId: marketplaceId(root, 'project_id', 'installed marketplace project'),
    projectSlug: expectString(root, 'project_slug', 'installed marketplace project'),
    projectTitle: expectString(root, 'project_title', 'installed marketplace project'),
    versionId: marketplaceId(root, 'version_id', 'installed marketplace project'),
    versionNumber: expectString(root, 'version_number', 'installed marketplace project'),
    filename,
  };
}

export function parseMarketplaceInstallResult(value: unknown): MarketplaceInstallResult {
  const root = expectRecord(value, 'marketplace installation result');
  if (nonnegative(root, 'schema_version', 'marketplace installation result') !== 1) throw new ApiError('Marketplace installation returned an unsupported schema.');
  const installedProjects = expectArray(root, 'installed_projects', 'marketplace installation result', 32).map(parseInstalledProject);
  const dependencyCount = expectNumber(root, 'dependency_count', 'marketplace installation result', { integer: true, minimum: 0, maximum: 31 });
  if (installedProjects.length === 0 || dependencyCount !== installedProjects.length - 1) {
    throw new ApiError('Marketplace installation returned an inconsistent dependency count.');
  }
  const backupId = expectString(root, 'backup_id', 'marketplace installation result');
  if (!/^\d{1,20}$/u.test(backupId)) throw new ApiError('Marketplace installation returned an invalid backup ID.');
  const serverWasRunning = boolean(root, 'server_was_running', 'marketplace installation result');
  const restartRequired = boolean(root, 'restart_required', 'marketplace installation result');
  if (restartRequired === serverWasRunning) throw new ApiError('Marketplace installation returned inconsistent restart state.');
  const runtimeValidationPerformed = boolean(root, 'runtime_validation_performed', 'marketplace installation result');
  const rollbackOnFailedStartup = boolean(root, 'rollback_on_failed_startup', 'marketplace installation result');
  if (runtimeValidationPerformed !== serverWasRunning || rollbackOnFailedStartup !== runtimeValidationPerformed) {
    throw new ApiError('Marketplace installation returned inconsistent runtime validation guarantees.');
  }
  return {
    instanceId: expectString(root, 'instance_id', 'marketplace installation result'),
    projectId: marketplaceId(root, 'project_id', 'marketplace installation result'),
    projectSlug: expectString(root, 'project_slug', 'marketplace installation result'),
    projectTitle: expectString(root, 'project_title', 'marketplace installation result'),
    versionId: marketplaceId(root, 'version_id', 'marketplace installation result'),
    versionNumber: expectString(root, 'version_number', 'marketplace installation result'),
    installedProjects,
    dependencyCount,
    optionalDependenciesNotInstalled: textList(root, 'optional_dependencies_not_installed', 'marketplace installation result', 4_096),
    backupId,
    serverWasRunning,
    restartRequired,
    runtimeValidationPerformed,
    rollbackOnFailedStartup,
  };
}

function parseInstallDispatch(value: unknown): MarketplaceInstallDispatch {
  const root = expectRecord(value, 'marketplace installation dispatch');
  return { jobId: uuid(root, 'job_id', 'marketplace installation dispatch'), reused: boolean(root, 'reused', 'marketplace installation dispatch') };
}

export function parseMarketplaceInstallJob(value: unknown): MarketplaceInstallJob {
  const root = expectRecord(value, 'marketplace installation job');
  const status = literal(root, 'status', 'marketplace installation job', ['queued', 'running', 'complete', 'failed'] as const);
  const result = status === 'complete' ? parseMarketplaceInstallResult(root.result) : null;
  if (status !== 'complete' && root.result !== null) throw new ApiError('Marketplace installation job returned a result before completion.');
  const error = nullableText(root, 'error', 'marketplace installation job', 8_192);
  if ((status === 'failed') !== (error !== null)) throw new ApiError('Marketplace installation job returned inconsistent failure state.');
  return {
    id: uuid(root, 'id', 'marketplace installation job'),
    kind: literal(root, 'kind', 'marketplace installation job', ['server_marketplace_install'] as const),
    status,
    stage: expectString(root, 'stage', 'marketplace installation job'),
    progressPercent: expectNumber(root, 'progress_percent', 'marketplace installation job', { integer: true, minimum: 0, maximum: 100 }),
    createdAtUnixMs: nonnegative(root, 'created_at_unix_ms', 'marketplace installation job'),
    updatedAtUnixMs: nonnegative(root, 'updated_at_unix_ms', 'marketplace installation job'),
    result,
    error,
  };
}

function validateMarketplaceId(value: string, label: string): void {
  if (!/^[A-Za-z0-9_-]{1,64}$/u.test(value)) throw new ApiError(`The marketplace ${label} ID is invalid.`);
}

export function searchMarketplace(instanceId: string, query: string, offset: number, limit: number, csrfToken: string, signal?: AbortSignal): Promise<MarketplaceSearchPage> {
  const trimmed = query.trim();
  if (new TextEncoder().encode(trimmed).length > 120 || Array.from(trimmed).some((character) => /\p{Cc}/u.test(character))) throw new ApiError('Marketplace search text is invalid.');
  if (!Number.isInteger(offset) || offset < 0 || offset > 10_000 || !Number.isInteger(limit) || limit < 1 || limit > 50) throw new ApiError('Marketplace pagination is invalid.');
  const params = new URLSearchParams({ query: trimmed, offset: String(offset), limit: String(limit) });
  return requestJson(`/api/v1/servers/${encodeURIComponent(instanceId)}/marketplace/search?${params.toString()}`, parseMarketplaceSearchPage, { csrfToken, signal, timeoutMs: 25_000 });
}

export function getMarketplaceProject(instanceId: string, projectId: string, csrfToken: string, signal?: AbortSignal): Promise<MarketplaceProjectDetail> {
  validateMarketplaceId(projectId, 'project');
  return requestJson(`/api/v1/servers/${encodeURIComponent(instanceId)}/marketplace/projects/${encodeURIComponent(projectId)}`, parseMarketplaceProjectDetail, { csrfToken, signal, timeoutMs: 25_000 });
}

export function installMarketplaceProject(instanceId: string, projectId: string, versionId: string | null, csrfToken: string): Promise<MarketplaceInstallDispatch> {
  validateMarketplaceId(projectId, 'project');
  if (versionId !== null) validateMarketplaceId(versionId, 'version');
  return requestJson(`/api/v1/servers/${encodeURIComponent(instanceId)}/marketplace/install`, parseInstallDispatch, { method: 'POST', body: { project_id: projectId, version_id: versionId }, csrfToken, timeoutMs: 25_000 });
}

export function getMarketplaceInstallJob(jobId: string, csrfToken: string, signal?: AbortSignal): Promise<MarketplaceInstallJob> {
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(jobId)) throw new ApiError('The marketplace job ID is invalid.');
  return requestJson(`/api/v1/jobs/${encodeURIComponent(jobId)}`, parseMarketplaceInstallJob, { csrfToken, signal, timeoutMs: 25_000 });
}
