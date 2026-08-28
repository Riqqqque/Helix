import {
  ApiError,
  expectArray,
  expectNumber,
  expectRecord,
  expectString,
  requestJson,
} from './api';
import { parseServerAppearance, type ServerAppearance } from './server-appearance-api';

export interface HostInventory {
  disks: BlockDevice[];
  mounts: MountInfo[];
  interfaces: InterfaceInfo[];
  routes: RouteInfo[];
  listeners: ListenerInfo[];
  services: ServiceInfo[];
  processes: ProcessInfo[];
  loadAverage: [number, number, number];
  collectedAtUnixMs: number;
}

export interface BlockDevice {
  name: string;
  path: string | null;
  parent: string | null;
  deviceType: string;
  sizeBytes: number;
  fileSystem: string | null;
  label: string | null;
  mountPoints: string[];
  model: string | null;
  serial: string | null;
  transport: string | null;
  rotational: boolean;
  readOnly: boolean;
  hotplug: boolean;
}

export interface MountInfo {
  target: string;
  source: string;
  fileSystem: string;
  sizeBytes: number;
  usedBytes: number;
  availableBytes: number;
  usePercent: number;
  readOnly: boolean;
}

export interface InterfaceInfo {
  name: string;
  state: string;
  mac: string | null;
  mtu: number;
  addresses: Array<{
    family: string;
    address: string;
    prefixLength: number;
    scope: string;
  }>;
  receivedBytes: number;
  transmittedBytes: number;
  receivedPackets: number;
  transmittedPackets: number;
  receivedErrors: number;
  transmittedErrors: number;
}

export interface RouteInfo {
  destination: string;
  gateway: string | null;
  interface: string | null;
  source: string | null;
  protocol: string | null;
  metric: number | null;
  linkDown: boolean;
}

export interface ListenerInfo {
  protocol: string;
  address: string;
  port: number;
  process: string | null;
}

export interface ServiceInfo {
  unit: string;
  active: string;
  state: string;
  description: string;
}

export interface ProcessInfo {
  pid: number;
  user: string;
  name: string;
  cpuPercent: number;
  residentBytes: number;
  uptimeSeconds: number;
}

export type FileKind = 'directory' | 'file' | 'symlink' | 'other';

export interface FileEntry {
  name: string;
  path: string;
  kind: FileKind;
  sizeBytes: number;
  modifiedUnixMs: number | null;
  permissions: string;
  ownerUid: number;
  ownerGid: number;
  writable: boolean;
  restricted: boolean;
  symlinkTarget: string | null;
}

export interface DirectoryListing {
  path: string;
  parent: string | null;
  writable: boolean;
  entries: FileEntry[];
  omittedEntries: number;
  totalEntries: number;
  nextCursor: string | null;
  hasMore: boolean;
  pageLimit: number;
}

export interface TextFile {
  path: string;
  content: string;
  sizeBytes: number;
  modifiedUnixMs: number | null;
}

export type ServerStatus = 'online' | 'offline' | 'manager_stopped';

export interface ManagedServer {
  id: string;
  name: string;
  instanceName: string;
  software: string;
  version: string;
  status: ServerStatus;
  panelRunning: boolean;
  startOnBoot: boolean;
  playersOnline: number;
  maxPlayers: number;
  cpuPercent: number;
  memoryUsedMb: number;
  memoryLimitMb: number;
  tps: number | null;
  managerPanelPort: number;
  panelPort: number;
  gamePort: number | null;
  path: string;
  warnings: string[];
  manager: 'helix' | 'amp_import';
  executionBackend: 'docker' | 'external';
  appearance: ServerAppearance;
}

export type ServerAction = 'start' | 'stop' | 'restart' | 'update' | 'backup';
export type MinecraftSoftware = 'custom' | 'vanilla' | 'paper' | 'purpur' | 'folia' | 'fabric' | 'neoforge';

export interface MinecraftCreateInput {
  name: string;
  software: MinecraftSoftware;
  version: string;
  memory_mb: number;
  max_players: number;
  game_port?: number;
  network_exposure: 'private' | 'public';
  start_on_boot: boolean;
  eula_accepted: boolean;
  custom_jar?: {
    source_path: string;
    java_version: 17 | 21 | 25;
  };
}

export interface GamePortRange {
  start: number;
  end: number;
}

export interface GamePortPolicy {
  ranges: GamePortRange[];
  ports: number[];
  autoForwardOnCreate: boolean;
  capacity: number;
  assignedPorts: number[];
  availableCount: number;
  nextAvailablePort: number | null;
}

export interface BrokerJob {
  id: string;
  kind: string;
  status: 'queued' | 'running' | 'complete' | 'failed';
  stage: string;
  progressPercent: number;
  createdAtUnixMs: number;
  updatedAtUnixMs: number;
  result: unknown;
  error: string | null;
}

export type NativeServerStatus = 'online' | 'starting' | 'stopped';
export type MinecraftGameMode = 'survival' | 'creative' | 'adventure' | 'spectator';
export type MinecraftDifficulty = 'peaceful' | 'easy' | 'normal' | 'hard';
export type MinecraftSettingField =
  | 'motd'
  | 'game_mode'
  | 'difficulty'
  | 'max_players'
  | 'view_distance'
  | 'simulation_distance'
  | 'player_idle_timeout'
  | 'online_mode'
  | 'pvp'
  | 'allow_flight'
  | 'white_list'
  | 'enforce_white_list'
  | 'spawn_protection';

export interface MinecraftRestartBehavior {
  activation: 'server_restart';
  restartRequiredFields: MinecraftSettingField[];
  message: string;
}

export interface MinecraftSettings {
  expectedRevision: string;
  motd: string;
  gameMode: MinecraftGameMode;
  difficulty: MinecraftDifficulty;
  maxPlayers: number;
  viewDistance: number;
  simulationDistance: number;
  playerIdleTimeout: number;
  onlineMode: boolean;
  pvp: boolean;
  allowFlight: boolean;
  whiteList: boolean;
  enforceWhiteList: boolean;
  spawnProtection: number;
  restartBehavior: MinecraftRestartBehavior;
}

export interface MinecraftSettingsSaveResult {
  changed: boolean;
  restartRequired: boolean;
  changedFields: MinecraftSettingField[];
  settings: MinecraftSettings;
}

export interface NativeServerDetail {
  id: string;
  name: string;
  instanceName: string;
  software: string;
  minecraftVersion: string;
  build: string;
  javaVersion: number;
  runtimeImage: string;
  artifactSha256: string;
  memoryLimitMb: number;
  gamePort: number;
  startOnBoot: boolean;
  createdAtUnixMs: number;
  dataPath: string;
  diskBytes: number;
  status: NativeServerStatus;
  playersOnline: number;
  maxPlayers: number;
  cpuPercent: number;
  memoryUsedMb: number;
  containerState: Record<string, unknown>;
  settings: MinecraftSettings;
  consoleHistory: {
    persistent: boolean;
    retentionBytes: number;
    retentionFiles: number;
    scope: 'per_server';
  };
  capabilities: string[];
}

export interface ServerLogSnapshot {
  instanceId: string;
  lines: string[];
  collectedAtUnixMs: number;
}

export interface ConsoleResult {
  instanceId: string;
  command: string;
  response: string;
  historyRecorded: boolean;
  completedAtUnixMs: number;
}

export interface ServerBackup {
  id: string;
  createdAtUnixMs: number;
  sizeBytes: number;
  definitionPresent: boolean;
}

export interface ServerBackupCatalog {
  instanceId: string;
  backups: ServerBackup[];
  trash: ServerBackupTrash[];
  trashPolicy: ServerBackupTrashPolicy;
}

export interface ServerBackupTrash {
  trashId: string;
  trashedAtUnixMs: number;
  undoAvailable: boolean;
  sizeBytes: number;
  definitionPresent: boolean;
}

export interface ServerBackupTrashPolicy {
  note: string;
}

export interface BackupTrashResult {
  trashId: string;
}

export interface BackupTrashRestoreResult {
  trashId: string;
}

export interface TrashedNativeServer {
  trashId: string;
  instanceId: string;
  name: string;
  software: string;
  minecraftVersion: string;
  gamePort: number;
  trashedAtUnixMs: number;
  dataPresent: boolean;
  backupsPreserved: boolean;
}

export interface TrashedNativeServerCatalog {
  servers: TrashedNativeServer[];
  policy: { recoverable: true; automaticPurge: false; note: string };
}

export interface JobDispatch {
  jobId: string | null;
}

function optionalString(record: Record<string, unknown>, key: string): string | null {
  const value = record[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'string' || value.length > 4096) {
    throw new Error(`Invalid ${key}`);
  }
  return value;
}

function boolean(record: Record<string, unknown>, key: string): boolean {
  const value = record[key];
  if (typeof value !== 'boolean') throw new Error(`Invalid ${key}`);
  return value;
}

function number(record: Record<string, unknown>, key: string): number {
  return expectNumber(record, key, key, { minimum: 0 });
}

function array(
  record: Record<string, unknown>,
  key: string,
  context: string,
  maximumLength = 2_048,
): unknown[] {
  return expectArray(record, key, context, maximumLength);
}

function nullableNumber(record: Record<string, unknown>, key: string): number | null {
  const value = record[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'number' || !Number.isFinite(value)) throw new Error(`Invalid ${key}`);
  return value;
}

function parseBlockDevice(value: unknown): BlockDevice {
  const item = expectRecord(value, 'block device');
  return {
    name: expectString(item, 'name', 'block device'),
    path: optionalString(item, 'path'),
    parent: optionalString(item, 'parent'),
    deviceType: expectString(item, 'device_type', 'block device'),
    sizeBytes: number(item, 'size_bytes'),
    fileSystem: optionalString(item, 'file_system'),
    label: optionalString(item, 'label'),
    mountPoints: array(item, 'mount_points', 'block device', 32).map((entry) => {
      if (typeof entry !== 'string') throw new Error('Invalid mount point');
      return entry;
    }),
    model: optionalString(item, 'model'),
    serial: optionalString(item, 'serial'),
    transport: optionalString(item, 'transport'),
    rotational: boolean(item, 'rotational'),
    readOnly: boolean(item, 'read_only'),
    hotplug: boolean(item, 'hotplug'),
  };
}

function parseMount(value: unknown): MountInfo {
  const item = expectRecord(value, 'mount');
  return {
    target: expectString(item, 'target', 'mount'),
    source: expectString(item, 'source', 'mount'),
    fileSystem: expectString(item, 'file_system', 'mount'),
    sizeBytes: number(item, 'size_bytes'),
    usedBytes: number(item, 'used_bytes'),
    availableBytes: number(item, 'available_bytes'),
    usePercent: number(item, 'use_percent'),
    readOnly: boolean(item, 'read_only'),
  };
}

function parseInterface(value: unknown): InterfaceInfo {
  const item = expectRecord(value, 'network interface');
  return {
    name: expectString(item, 'name', 'network interface'),
    state: expectString(item, 'state', 'network interface'),
    mac: optionalString(item, 'mac'),
    mtu: number(item, 'mtu'),
    addresses: array(item, 'addresses', 'network interface', 32).map((entry) => {
      const address = expectRecord(entry, 'network address');
      return {
        family: expectString(address, 'family', 'network address'),
        address: expectString(address, 'address', 'network address'),
        prefixLength: number(address, 'prefix_length'),
        scope: expectString(address, 'scope', 'network address'),
      };
    }),
    receivedBytes: number(item, 'received_bytes'),
    transmittedBytes: number(item, 'transmitted_bytes'),
    receivedPackets: number(item, 'received_packets'),
    transmittedPackets: number(item, 'transmitted_packets'),
    receivedErrors: number(item, 'received_errors'),
    transmittedErrors: number(item, 'transmitted_errors'),
  };
}

export function parseHostInventory(value: unknown): HostInventory {
  const root = expectRecord(value, 'host inventory');
  const load = array(root, 'load_average', 'host inventory', 3).map((entry) => {
    if (typeof entry !== 'number' || !Number.isFinite(entry)) throw new Error('Invalid load average');
    return entry;
  });
  if (load.length !== 3) throw new Error('Invalid load average');
  return {
    disks: array(root, 'disks', 'host inventory', 128).map(parseBlockDevice),
    mounts: array(root, 'mounts', 'host inventory', 128).map(parseMount),
    interfaces: array(root, 'interfaces', 'host inventory', 128).map(parseInterface),
    routes: array(root, 'routes', 'host inventory', 256).map((entry) => {
      const route = expectRecord(entry, 'route');
      return {
        destination: expectString(route, 'destination', 'route'),
        gateway: optionalString(route, 'gateway'),
        interface: optionalString(route, 'interface'),
        source: optionalString(route, 'source'),
        protocol: optionalString(route, 'protocol'),
        metric: nullableNumber(route, 'metric'),
        linkDown: boolean(route, 'link_down'),
      };
    }),
    listeners: array(root, 'listeners', 'host inventory', 512).map((entry) => {
      const listener = expectRecord(entry, 'listener');
      return {
        protocol: expectString(listener, 'protocol', 'listener'),
        address: expectString(listener, 'address', 'listener'),
        port: number(listener, 'port'),
        process: optionalString(listener, 'process'),
      };
    }),
    services: array(root, 'services', 'host inventory', 256).map((entry) => {
      const service = expectRecord(entry, 'service');
      return {
        unit: expectString(service, 'unit', 'service'),
        active: expectString(service, 'active', 'service'),
        state: expectString(service, 'state', 'service'),
        description: expectString(service, 'description', 'service'),
      };
    }),
    processes: array(root, 'processes', 'host inventory', 32).map((entry) => {
      const process = expectRecord(entry, 'process');
      return {
        pid: number(process, 'pid'),
        user: expectString(process, 'user', 'process'),
        name: expectString(process, 'name', 'process'),
        cpuPercent: number(process, 'cpu_percent'),
        residentBytes: number(process, 'resident_bytes'),
        uptimeSeconds: number(process, 'uptime_seconds'),
      };
    }),
    loadAverage: load as [number, number, number],
    collectedAtUnixMs: number(root, 'collected_at_unix_ms'),
  };
}

export function parseDirectoryListing(value: unknown): DirectoryListing {
  const root = expectRecord(value, 'directory listing');
  const totalEntries = expectNumber(root, 'total_entries', 'directory listing', { integer: true, minimum: 0 });
  const pageLimit = expectNumber(root, 'page_limit', 'directory listing', { integer: true, minimum: 25, maximum: 200 });
  const nextCursor = optionalString(root, 'next_cursor');
  const hasMore = boolean(root, 'has_more');
  const entries = array(root, 'entries', 'directory listing', pageLimit).map((entry) => {
    const item = expectRecord(entry, 'file entry');
    const kind = expectString(item, 'kind', 'file entry') as FileKind;
    if (!['directory', 'file', 'symlink', 'other'].includes(kind)) {
      throw new Error('Invalid file kind');
    }
    return {
      name: expectString(item, 'name', 'file entry'),
      path: expectString(item, 'path', 'file entry'),
      kind,
      sizeBytes: number(item, 'size_bytes'),
      modifiedUnixMs: nullableNumber(item, 'modified_unix_ms'),
      permissions: expectString(item, 'permissions', 'file entry'),
      ownerUid: number(item, 'owner_uid'),
      ownerGid: number(item, 'owner_gid'),
      writable: boolean(item, 'writable'),
      restricted: boolean(item, 'restricted'),
      symlinkTarget: optionalString(item, 'symlink_target'),
    };
  });
  if (hasMore !== (nextCursor !== null) || entries.length > totalEntries) {
    throw new Error('Invalid directory pagination');
  }
  return {
    path: expectString(root, 'path', 'directory listing'),
    parent: optionalString(root, 'parent'),
    writable: boolean(root, 'writable'),
    omittedEntries: expectNumber(root, 'omitted_entries', 'directory listing', { integer: true, minimum: 0 }),
    totalEntries,
    nextCursor,
    hasMore,
    pageLimit,
    entries,
  };
}

function parseTextFile(value: unknown): TextFile {
  const root = expectRecord(value, 'text file');
  return {
    path: expectString(root, 'path', 'text file'),
    content:
      typeof root.content === 'string'
        ? root.content
        : (() => {
            throw new Error('Invalid text content');
          })(),
    sizeBytes: number(root, 'size_bytes'),
    modifiedUnixMs: nullableNumber(root, 'modified_unix_ms'),
  };
}

export function parseServers(value: unknown): ManagedServer[] {
  if (!Array.isArray(value)) throw new Error('Invalid server list');
  return value.map((entry) => {
    const item = expectRecord(entry, 'server');
    const status = expectString(item, 'status', 'server') as ServerStatus;
    if (!['online', 'offline', 'manager_stopped'].includes(status)) {
      throw new Error('Invalid server status');
    }
    const manager = expectString(item, 'manager', 'server');
    const executionBackend = expectString(item, 'execution_backend', 'server');
    if (manager !== 'helix' && manager !== 'amp_import') {
      throw new Error('Invalid server manager');
    }
    if (executionBackend !== 'docker' && executionBackend !== 'external') {
      throw new Error('Invalid execution backend');
    }
    return {
      id: expectString(item, 'id', 'server'),
      name: expectString(item, 'name', 'server'),
      instanceName: expectString(item, 'instance_name', 'server'),
      software: expectString(item, 'software', 'server'),
      version: expectString(item, 'version', 'server'),
      status,
      panelRunning: boolean(item, 'panel_running'),
      startOnBoot: boolean(item, 'start_on_boot'),
      playersOnline: number(item, 'players_online'),
      maxPlayers: number(item, 'max_players'),
      cpuPercent: number(item, 'cpu_percent'),
      memoryUsedMb: number(item, 'memory_used_mb'),
      memoryLimitMb: number(item, 'memory_limit_mb'),
      tps: nullableNumber(item, 'tps'),
      managerPanelPort: number(item, 'manager_panel_port'),
      panelPort: number(item, 'panel_port'),
      gamePort: nullableNumber(item, 'game_port'),
      path: expectString(item, 'path', 'server'),
      warnings: array(item, 'warnings', 'server', 64).map((warning) => {
        if (typeof warning !== 'string') throw new Error('Invalid server warning');
        return warning;
      }),
      manager,
      executionBackend,
      appearance: parseServerAppearance(item.appearance),
    };
  });
}

function parseJob(value: unknown): BrokerJob {
  const root = expectRecord(value, 'job');
  const status = expectString(root, 'status', 'job') as BrokerJob['status'];
  if (!['queued', 'running', 'complete', 'failed'].includes(status)) throw new Error('Invalid job status');
  return {
    id: expectString(root, 'id', 'job'),
    kind: expectString(root, 'kind', 'job'),
    status,
    stage: expectString(root, 'stage', 'job'),
    progressPercent: number(root, 'progress_percent'),
    createdAtUnixMs: number(root, 'created_at_unix_ms'),
    updatedAtUnixMs: number(root, 'updated_at_unix_ms'),
    result: root.result,
    error: optionalString(root, 'error'),
  };
}

const minecraftSettingFields = new Set<MinecraftSettingField>([
  'motd', 'game_mode', 'difficulty', 'max_players', 'view_distance',
  'simulation_distance', 'player_idle_timeout', 'online_mode', 'pvp',
  'allow_flight', 'white_list', 'enforce_white_list', 'spawn_protection',
]);

function parseSettingFields(value: unknown, context: string): MinecraftSettingField[] {
  if (!Array.isArray(value) || value.length > minecraftSettingFields.size) {
    throw new Error(`Invalid ${context}`);
  }
  const fields = value.map((entry) => {
    if (typeof entry !== 'string' || !minecraftSettingFields.has(entry as MinecraftSettingField)) {
      throw new Error(`Invalid ${context}`);
    }
    return entry as MinecraftSettingField;
  });
  if (new Set(fields).size !== fields.length) throw new Error(`Invalid ${context}`);
  return fields;
}

function parseRestartBehavior(value: unknown): MinecraftRestartBehavior {
  const root = expectRecord(value, 'restart behavior');
  const activation = expectString(root, 'activation', 'restart behavior');
  if (activation !== 'server_restart') throw new Error('Invalid restart activation');
  return {
    activation,
    restartRequiredFields: parseSettingFields(
      root.restart_required_fields,
      'restart-required fields',
    ),
    message: expectString(root, 'message', 'restart behavior'),
  };
}

function parseMinecraftSettings(value: unknown): MinecraftSettings {
  const root = expectRecord(value, 'Minecraft settings');
  const gameMode = expectString(root, 'game_mode', 'Minecraft settings') as MinecraftGameMode;
  const difficulty = expectString(root, 'difficulty', 'Minecraft settings') as MinecraftDifficulty;
  if (!['survival', 'creative', 'adventure', 'spectator'].includes(gameMode)) {
    throw new Error('Invalid game mode');
  }
  if (!['peaceful', 'easy', 'normal', 'hard'].includes(difficulty)) {
    throw new Error('Invalid difficulty');
  }
  return {
    expectedRevision: expectString(root, 'expected_revision', 'Minecraft settings'),
    motd: expectString(root, 'motd', 'Minecraft settings'),
    gameMode,
    difficulty,
    maxPlayers: number(root, 'max_players'),
    viewDistance: number(root, 'view_distance'),
    simulationDistance: number(root, 'simulation_distance'),
    playerIdleTimeout: number(root, 'player_idle_timeout'),
    onlineMode: boolean(root, 'online_mode'),
    pvp: boolean(root, 'pvp'),
    allowFlight: boolean(root, 'allow_flight'),
    whiteList: boolean(root, 'white_list'),
    enforceWhiteList: boolean(root, 'enforce_white_list'),
    spawnProtection: number(root, 'spawn_protection'),
    restartBehavior: parseRestartBehavior(root.restart_behavior),
  };
}

export function parseMinecraftSettingsSaveResult(value: unknown): MinecraftSettingsSaveResult {
  const root = expectRecord(value, 'saved server settings');
  return {
    changed: boolean(root, 'changed'),
    restartRequired: boolean(root, 'restart_required'),
    changedFields: parseSettingFields(root.changed_fields, 'changed fields'),
    settings: parseMinecraftSettings(root.settings),
  };
}

function parseNativeServerDetail(value: unknown): NativeServerDetail {
  const root = expectRecord(value, 'server detail');
  const status = expectString(root, 'status', 'server detail') as NativeServerStatus;
  if (!['online', 'starting', 'stopped'].includes(status)) throw new Error('Invalid native server status');
  const containerState = expectRecord(root.container_state, 'container state');
  const consoleHistory = expectRecord(root.console_history, 'console history configuration');
  const consoleHistoryScope = expectString(consoleHistory, 'scope', 'console history configuration');
  if (consoleHistoryScope !== 'per_server') throw new Error('Invalid console history scope');
  return {
    id: expectString(root, 'id', 'server detail'),
    name: expectString(root, 'name', 'server detail'),
    instanceName: expectString(root, 'instance_name', 'server detail'),
    software: expectString(root, 'software', 'server detail'),
    minecraftVersion: expectString(root, 'minecraft_version', 'server detail'),
    build: expectString(root, 'build', 'server detail'),
    javaVersion: number(root, 'java_version'),
    runtimeImage: expectString(root, 'runtime_image', 'server detail'),
    artifactSha256: expectString(root, 'artifact_sha256', 'server detail'),
    memoryLimitMb: number(root, 'memory_limit_mb'),
    gamePort: number(root, 'game_port'),
    startOnBoot: boolean(root, 'start_on_boot'),
    createdAtUnixMs: number(root, 'created_at_unix_ms'),
    dataPath: expectString(root, 'data_path', 'server detail'),
    diskBytes: number(root, 'disk_bytes'),
    status,
    playersOnline: number(root, 'players_online'),
    maxPlayers: number(root, 'max_players'),
    cpuPercent: number(root, 'cpu_percent'),
    memoryUsedMb: number(root, 'memory_used_mb'),
    containerState,
    settings: parseMinecraftSettings(root.settings),
    consoleHistory: {
      persistent: boolean(consoleHistory, 'persistent'),
      retentionBytes: number(consoleHistory, 'retention_bytes'),
      retentionFiles: number(consoleHistory, 'retention_files'),
      scope: consoleHistoryScope,
    },
    capabilities: array(root, 'capabilities', 'server detail', 32).map((entry) => {
      if (typeof entry !== 'string') throw new Error('Invalid server capability');
      return entry;
    }),
  };
}

function parseServerLogs(value: unknown): ServerLogSnapshot {
  const root = expectRecord(value, 'server logs');
  return {
    instanceId: expectString(root, 'instance_id', 'server logs'),
    lines: array(root, 'lines', 'server logs', 1_000).map((line) => {
      if (typeof line !== 'string') throw new Error('Invalid server log line');
      return line;
    }),
    collectedAtUnixMs: number(root, 'collected_at_unix_ms'),
  };
}

function parseConsoleResult(value: unknown): ConsoleResult {
  const root = expectRecord(value, 'console response');
  if (typeof root.response !== 'string') throw new Error('Invalid console response text');
  return {
    instanceId: expectString(root, 'instance_id', 'console response'),
    command: expectString(root, 'command', 'console response'),
    response: root.response,
    historyRecorded: boolean(root, 'history_recorded'),
    completedAtUnixMs: number(root, 'completed_at_unix_ms'),
  };
}

function backupId(record: Record<string, unknown>, key: string, context: string): string {
  const value = expectString(record, key, context);
  if (!/^\d{1,20}$/u.test(value)) throw new Error(`Invalid ${context} ID`);
  return value;
}

function trashId(record: Record<string, unknown>, key: string, context: string): string {
  const value = expectString(record, key, context);
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(value)) {
    throw new Error(`Invalid ${context} trash ID`);
  }
  return value;
}

export function parseBackupCatalog(value: unknown): ServerBackupCatalog {
  const root = expectRecord(value, 'backup catalog');
  const policy = expectRecord(root.trash_policy, 'backup trash policy');
  return {
    instanceId: expectString(root, 'instance_id', 'backup catalog'),
    backups: array(root, 'backups', 'backup catalog', 2_000).map((entry) => {
      const backup = expectRecord(entry, 'server backup');
      return {
        id: backupId(backup, 'id', 'server backup'),
        createdAtUnixMs: number(backup, 'created_at_unix_ms'),
        sizeBytes: number(backup, 'size_bytes'),
        definitionPresent: boolean(backup, 'definition_present'),
      };
    }),
    trash: array(root, 'trash', 'backup catalog', 2_048).map((entry) => {
      const trash = expectRecord(entry, 'deleted backup');
      return {
        trashId: trashId(trash, 'trash_id', 'deleted backup'),
        trashedAtUnixMs: number(trash, 'trashed_at_unix_ms'),
        undoAvailable: boolean(trash, 'undo_available'),
        sizeBytes: number(trash, 'size_bytes'),
        definitionPresent: boolean(trash, 'definition_present'),
      };
    }),
    trashPolicy: {
      note: expectString(policy, 'note', 'backup trash policy'),
    },
  };
}

function parseBackupTrashResult(value: unknown): BackupTrashResult {
  const root = expectRecord(value, 'backup deletion');
  return {
    trashId: trashId(root, 'trash_id', 'backup deletion'),
  };
}

function parseBackupTrashRestoreResult(value: unknown): BackupTrashRestoreResult {
  const root = expectRecord(value, 'deleted backup restore');
  return {
    trashId: trashId(root, 'trash_id', 'deleted backup restore'),
  };
}

function parseJobDispatch(value: unknown): JobDispatch {
  const root = expectRecord(value, 'job dispatch');
  return { jobId: optionalString(root, 'job_id') };
}

const identity = (value: unknown): unknown => value;

export function getHostInventory(csrfToken: string, signal?: AbortSignal): Promise<HostInventory> {
  return requestJson('/api/v1/host/inventory', parseHostInventory, { csrfToken, signal });
}

export function getDirectory(
  path: string,
  csrfToken: string,
  cursor: string | null = null,
  limit = 50,
  signal?: AbortSignal,
): Promise<DirectoryListing> {
  const query = new URLSearchParams({ path, limit: String(limit) });
  if (cursor !== null) query.set('cursor', cursor);
  return requestJson(`/api/v1/files?${query.toString()}`, parseDirectoryListing, {
    csrfToken,
    signal,
    timeoutMs: 30_000,
  });
}

export function createDirectory(parent: string, name: string, csrfToken: string): Promise<unknown> {
  return requestJson('/api/v1/files/directory', identity, { method: 'POST', body: { parent, name }, csrfToken });
}

export function createFile(parent: string, name: string, csrfToken: string): Promise<unknown> {
  return requestJson('/api/v1/files/file', identity, { method: 'POST', body: { parent, name }, csrfToken });
}

export function readTextFile(path: string, csrfToken: string): Promise<TextFile> {
  return requestJson('/api/v1/files/read', parseTextFile, { method: 'POST', body: { path }, csrfToken });
}

export function writeTextFile(file: TextFile, content: string, csrfToken: string): Promise<TextFile> {
  return requestJson('/api/v1/files/write', parseTextFile, {
    method: 'POST',
    body: { path: file.path, content, expected_modified_unix_ms: file.modifiedUnixMs },
    csrfToken,
    timeoutMs: 20_000,
  });
}

export function renameFile(path: string, newName: string, csrfToken: string): Promise<unknown> {
  return requestJson('/api/v1/files/rename', identity, { method: 'POST', body: { path, new_name: newName }, csrfToken });
}

export function trashFile(path: string, csrfToken: string): Promise<unknown> {
  return requestJson('/api/v1/files/trash', identity, { method: 'POST', body: { path }, csrfToken });
}

export function getServers(csrfToken: string, signal?: AbortSignal): Promise<ManagedServer[]> {
  return requestJson('/api/v1/servers', parseServers, { csrfToken, signal });
}

function parseTrashedNativeServerCatalog(value: unknown): TrashedNativeServerCatalog {
  const root = expectRecord(value, 'removed server catalog');
  if (number(root, 'schema_version') !== 1) throw new Error('Invalid removed server catalog schema');
  const policy = expectRecord(root.policy, 'removed server policy');
  if (!boolean(policy, 'recoverable') || boolean(policy, 'automatic_purge')) {
    throw new Error('Invalid removed server recovery policy');
  }
  return {
    servers: array(root, 'servers', 'removed server catalog', 512).map((entry) => {
      const item = expectRecord(entry, 'removed server');
      return {
        trashId: expectString(item, 'trash_id', 'removed server'),
        instanceId: expectString(item, 'instance_id', 'removed server'),
        name: expectString(item, 'name', 'removed server'),
        software: expectString(item, 'software', 'removed server'),
        minecraftVersion: expectString(item, 'minecraft_version', 'removed server'),
        gamePort: number(item, 'game_port'),
        trashedAtUnixMs: number(item, 'trashed_at_unix_ms'),
        dataPresent: boolean(item, 'data_present'),
        backupsPreserved: boolean(item, 'backups_preserved'),
      };
    }),
    policy: {
      recoverable: true,
      automaticPurge: false,
      note: expectString(policy, 'note', 'removed server policy'),
    },
  };
}

export function getTrashedNativeServers(csrfToken: string, signal?: AbortSignal): Promise<TrashedNativeServerCatalog> {
  return requestJson('/api/v1/servers/removed', parseTrashedNativeServerCatalog, { csrfToken, signal });
}

export function trashNativeServer(id: string, confirmationName: string, csrfToken: string): Promise<{ trashId: string }> {
  return requestJson(`/api/v1/servers/${encodeURIComponent(id)}/remove`, (value) => {
    const root = expectRecord(value, 'removed server result');
    if (!boolean(root, 'recoverable')) throw new Error('Server removal was not recoverable');
    return { trashId: expectString(root, 'trash_id', 'removed server result') };
  }, { method: 'POST', body: { confirmation_name: confirmationName }, csrfToken, timeoutMs: 90_000 });
}

export function restoreTrashedNativeServer(trashId: string, csrfToken: string): Promise<{ instanceId: string }> {
  return requestJson(`/api/v1/servers/removed/${encodeURIComponent(trashId)}/restore`, (value) => {
    const root = expectRecord(value, 'restored server result');
    if (!boolean(root, 'restored')) throw new Error('Server restore was not verified');
    return { instanceId: expectString(root, 'instance_id', 'restored server result') };
  }, { method: 'POST', body: {}, csrfToken, timeoutMs: 90_000 });
}

export function runServerAction(id: string, action: ServerAction, csrfToken: string): Promise<JobDispatch> {
  return requestJson(`/api/v1/servers/${encodeURIComponent(id)}/actions`, parseJobDispatch, {
    method: 'POST', body: { action }, csrfToken, timeoutMs: 60_000,
  });
}

export function createMinecraftServer(input: MinecraftCreateInput, csrfToken: string): Promise<{ jobId: string }> {
  return requestJson('/api/v1/servers/minecraft', (value) => {
    const root = expectRecord(value, 'Minecraft job');
    return { jobId: expectString(root, 'job_id', 'Minecraft job') };
  }, { method: 'POST', body: input, csrfToken, timeoutMs: 20_000 });
}

function parseGamePortPolicy(value: unknown): GamePortPolicy {
  const root = expectRecord(value, 'game port policy');
  if (number(root, 'schema_version') !== 1) throw new ApiError('Unsupported game port policy schema.');
  const policy = expectRecord(root.policy, 'game port policy details');
  if (expectString(policy, 'game', 'game port policy details') !== 'minecraft') {
    throw new ApiError('Game port policy returned the wrong game.');
  }
  const ranges = expectArray(policy, 'ranges', 'game port policy details', 32).map((value) => {
    const range = expectRecord(value, 'game port range');
    return { start: number(range, 'start'), end: number(range, 'end') };
  });
  return {
    ranges,
    ports: expectArray(policy, 'ports', 'game port policy details', 256).map((value) => {
      if (typeof value !== 'number' || !Number.isInteger(value)) throw new ApiError('Game port policy returned an invalid port.');
      return value;
    }),
    autoForwardOnCreate: boolean(policy, 'auto_forward_on_create'),
    capacity: number(root, 'capacity'),
    assignedPorts: expectArray(root, 'assigned_ports', 'game port policy', 4096).map((value) => {
      if (typeof value !== 'number' || !Number.isInteger(value)) throw new ApiError('Game port policy returned an invalid assigned port.');
      return value;
    }),
    availableCount: number(root, 'available_count'),
    nextAvailablePort: nullableNumber(root, 'next_available_port'),
  };
}

export function getMinecraftPortPolicy(csrfToken: string, signal?: AbortSignal): Promise<GamePortPolicy> {
  return requestJson('/api/v1/servers/port-policies/minecraft', parseGamePortPolicy, { csrfToken, signal });
}

export function saveMinecraftPortPolicy(
  input: Pick<GamePortPolicy, 'ranges' | 'ports' | 'autoForwardOnCreate'>,
  csrfToken: string,
): Promise<GamePortPolicy> {
  return requestJson('/api/v1/servers/port-policies/minecraft', parseGamePortPolicy, {
    method: 'PUT',
    body: {
      game: 'minecraft',
      ranges: input.ranges,
      ports: input.ports,
      auto_forward_on_create: input.autoForwardOnCreate,
    },
    csrfToken,
  });
}

export function setServerNetworkExposure(
  id: string,
  enabled: boolean,
  csrfToken: string,
): Promise<unknown> {
  return requestJson(`/api/v1/servers/${encodeURIComponent(id)}/network`, (value) => value, {
    method: 'PUT', body: { enabled }, csrfToken, timeoutMs: 30_000,
  });
}

export function getJob(jobId: string, csrfToken: string, signal?: AbortSignal): Promise<BrokerJob> {
  return requestJson(`/api/v1/jobs/${encodeURIComponent(jobId)}`, parseJob, { csrfToken, signal });
}

export function getServerDetail(id: string, csrfToken: string, signal?: AbortSignal): Promise<NativeServerDetail> {
  return requestJson(`/api/v1/servers/${encodeURIComponent(id)}`, parseNativeServerDetail, { csrfToken, signal, timeoutMs: 30_000 });
}

export function getServerLogs(id: string, csrfToken: string, signal?: AbortSignal): Promise<ServerLogSnapshot> {
  return requestJson(`/api/v1/servers/${encodeURIComponent(id)}/logs?lines=500`, parseServerLogs, { csrfToken, signal });
}

export function sendConsoleCommand(id: string, command: string, csrfToken: string): Promise<ConsoleResult> {
  return requestJson(`/api/v1/servers/${encodeURIComponent(id)}/console`, parseConsoleResult, {
    method: 'POST', body: { command }, csrfToken, timeoutMs: 15_000,
  });
}

export function getServerSettings(id: string, csrfToken: string, signal?: AbortSignal): Promise<MinecraftSettings> {
  return requestJson(`/api/v1/servers/${encodeURIComponent(id)}/settings`, parseMinecraftSettings, { csrfToken, signal });
}

export function saveServerSettings(id: string, settings: MinecraftSettings, csrfToken: string): Promise<MinecraftSettingsSaveResult> {
  return requestJson(`/api/v1/servers/${encodeURIComponent(id)}/settings`, parseMinecraftSettingsSaveResult, {
    method: 'POST',
    body: {
      expected_revision: settings.expectedRevision,
      motd: settings.motd,
      game_mode: settings.gameMode,
      difficulty: settings.difficulty,
      max_players: settings.maxPlayers,
      view_distance: settings.viewDistance,
      simulation_distance: settings.simulationDistance,
      player_idle_timeout: settings.playerIdleTimeout,
      online_mode: settings.onlineMode,
      pvp: settings.pvp,
      allow_flight: settings.allowFlight,
      white_list: settings.whiteList,
      enforce_white_list: settings.enforceWhiteList,
      spawn_protection: settings.spawnProtection,
    },
    csrfToken,
    timeoutMs: 20_000,
  });
}

export function getServerBackups(id: string, csrfToken: string, signal?: AbortSignal): Promise<ServerBackupCatalog> {
  return requestJson(`/api/v1/servers/${encodeURIComponent(id)}/backups`, parseBackupCatalog, { csrfToken, signal });
}

export function restoreServerBackup(id: string, backupId: string, csrfToken: string): Promise<JobDispatch> {
  return requestJson(
    `/api/v1/servers/${encodeURIComponent(id)}/backups/${encodeURIComponent(backupId)}/restore`,
    parseJobDispatch,
    { method: 'POST', body: {}, csrfToken, timeoutMs: 20_000 },
  );
}

export function trashServerBackup(
  id: string,
  backupIdValue: string,
  csrfToken: string,
): Promise<BackupTrashResult> {
  return requestJson(
    `/api/v1/servers/${encodeURIComponent(id)}/backups/${encodeURIComponent(backupIdValue)}`,
    parseBackupTrashResult,
    { method: 'DELETE', body: {}, csrfToken, timeoutMs: 20_000 },
  );
}

export function restoreTrashedServerBackup(
  id: string,
  trashIdValue: string,
  csrfToken: string,
): Promise<BackupTrashRestoreResult> {
  return requestJson(
    `/api/v1/servers/${encodeURIComponent(id)}/backups/trash/${encodeURIComponent(trashIdValue)}/restore`,
    parseBackupTrashRestoreResult,
    { method: 'POST', body: {}, csrfToken, timeoutMs: 20_000 },
  );
}
