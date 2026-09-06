import {
  ApiError,
  expectArray,
  expectBoolean,
  expectNumber,
  expectRecord,
  expectString,
  requestJson,
  type JsonRecord,
} from './api';
import type { MinecraftSoftware } from './control-api';

export type MigrateGame = 'minecraft' | 'vrising' | 'valheim' | 'terraria';
export type TerrariaMigrateSoftware = 'vanilla' | 'tmodloader';

export type ServerMigrateSource =
  | { kind: 'amp'; instance_id: string }
  | { kind: 'folder'; path: string };

export interface ServerMigratePreflight {
  schemaVersion: number;
  game: MigrateGame;
  sourceKind: 'amp' | 'folder';
  sourceId: string;
  sourceName: string;
  sourcePath: string;
  gameRoot: string;
  software: MinecraftSoftware | null;
  terrariaSoftware: TerrariaMigrateSoftware | null;
  copyServerJar: boolean;
  version: string;
  versionUsedLatest: boolean;
  memoryMb: number;
  maxPlayers: number;
  running: boolean;
  status: string;
  files: number;
  bytes: number;
  skipped: number;
  copies: string[];
  skips: string[];
  warning: string | null;
  blockers: string[];
  notes: string[];
}

export interface ServerMigrateInput {
  source: ServerMigrateSource;
  name: string;
  game?: MigrateGame;
  software?: MinecraftSoftware;
  version?: string;
  memory_mb: number;
  cpu_millis?: number;
  max_players: number;
  game_port?: number;
  query_port?: number;
  network_exposure: 'private' | 'public';
  start_on_boot: boolean;
  eula_accepted: boolean;
  source_stopped: boolean;
  copy_acknowledged: boolean;
  list_on_browser?: boolean;
}

const MIGRATE_GAMES: ReadonlyArray<MigrateGame> = ['minecraft', 'vrising', 'valheim', 'terraria'];
const MINECRAFT_SOFTWARE: ReadonlyArray<MinecraftSoftware> = [
  'custom',
  'vanilla',
  'paper',
  'purpur',
  'folia',
  'leaves',
  'fabric',
  'neoforge',
  'forge',
  'quilt',
  'pufferfish',
];

function optionalString(record: JsonRecord, key: string, context: string): string | null {
  const value = record[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'string' || value.length > 8_192) {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }
  return value;
}

function stringList(record: JsonRecord, key: string, context: string, maximum: number): string[] {
  return expectArray(record, key, context, maximum).map((entry) => {
    if (typeof entry !== 'string' || entry.length === 0 || entry.length > 1_024) {
      throw new ApiError(`${context} returned an invalid ${key} value.`);
    }
    return entry;
  });
}

export function parseMigratePreflight(value: unknown): ServerMigratePreflight {
  const root = expectRecord(value, 'copy preflight');
  const game = expectString(root, 'game', 'copy preflight') as MigrateGame;
  if (!MIGRATE_GAMES.includes(game)) {
    throw new ApiError('copy preflight returned an invalid game value.');
  }
  const sourceKind = expectString(root, 'source_kind', 'copy preflight');
  if (sourceKind !== 'amp' && sourceKind !== 'folder') {
    throw new ApiError('copy preflight returned an invalid source_kind value.');
  }
  const softwareRaw = optionalString(root, 'software', 'copy preflight');
  const software =
    softwareRaw === null
      ? null
      : (MINECRAFT_SOFTWARE.find((id) => id === softwareRaw) ?? null);
  if (softwareRaw !== null && software === null) {
    throw new ApiError('copy preflight returned an invalid software value.');
  }
  const terrariaRaw = optionalString(root, 'terraria_software', 'copy preflight');
  const terrariaSoftware: TerrariaMigrateSoftware | null =
    terrariaRaw === null
      ? null
      : terrariaRaw === 'vanilla' || terrariaRaw === 'tmodloader'
        ? terrariaRaw
        : (() => {
            throw new ApiError('copy preflight returned an invalid terraria_software value.');
          })();
  return {
    schemaVersion: expectNumber(root, 'schema_version', 'copy preflight', {
      integer: true,
      minimum: 1,
      maximum: 8,
    }),
    game,
    sourceKind,
    sourceId: expectString(root, 'source_id', 'copy preflight'),
    sourceName: expectString(root, 'source_name', 'copy preflight'),
    sourcePath: expectString(root, 'source_path', 'copy preflight'),
    gameRoot: expectString(root, 'game_root', 'copy preflight'),
    software,
    terrariaSoftware,
    copyServerJar: expectBoolean(root, 'copy_server_jar', 'copy preflight'),
    version: expectString(root, 'version', 'copy preflight'),
    versionUsedLatest: expectBoolean(root, 'version_used_latest', 'copy preflight'),
    memoryMb: expectNumber(root, 'memory_mb', 'copy preflight', {
      integer: true,
      minimum: 512,
      maximum: 24_576,
    }),
    maxPlayers: expectNumber(root, 'max_players', 'copy preflight', {
      integer: true,
      minimum: 1,
      maximum: 10_000,
    }),
    running: expectBoolean(root, 'running', 'copy preflight'),
    status: expectString(root, 'status', 'copy preflight'),
    files: expectNumber(root, 'files', 'copy preflight', { integer: true, minimum: 0 }),
    bytes: expectNumber(root, 'bytes', 'copy preflight', { integer: true, minimum: 0 }),
    skipped: expectNumber(root, 'skipped', 'copy preflight', { integer: true, minimum: 0 }),
    copies: stringList(root, 'copies', 'copy preflight', 64),
    skips: stringList(root, 'skips', 'copy preflight', 64),
    warning: optionalString(root, 'warning', 'copy preflight'),
    blockers: stringList(root, 'blockers', 'copy preflight', 16),
    notes: stringList(root, 'notes', 'copy preflight', 16),
  };
}

export function migrateServerPreflight(
  source: ServerMigrateSource,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<ServerMigratePreflight> {
  return requestJson('/api/v1/servers/migrate/preflight', parseMigratePreflight, {
    method: 'POST',
    body: source,
    csrfToken,
    signal,
    timeoutMs: 30_000,
  });
}

export function migrateServer(
  input: ServerMigrateInput,
  csrfToken: string,
): Promise<{ jobId: string }> {
  return requestJson(
    '/api/v1/servers/migrate',
    (value) => {
      const root = expectRecord(value, 'copy job');
      return { jobId: expectString(root, 'job_id', 'copy job') };
    },
    { method: 'POST', body: input, csrfToken, timeoutMs: 20_000 },
  );
}
