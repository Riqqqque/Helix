export type GameHostingAvailability = 'ready' | 'degraded' | 'unavailable';

export type GameHostingBlockerStatus = 'required' | 'in_progress' | 'ready';

export interface GameHostingBlocker {
  code: string;
  status: GameHostingBlockerStatus;
}

export interface GameHostingReadiness {
  schemaVersion: 1;
  availability: GameHostingAvailability;
  availableFeatures: string[];
  blockers: GameHostingBlocker[];
  collectedAtUnixMs: number;
}

export type GameInstanceStatus =
  | 'online'
  | 'starting'
  | 'stopping'
  | 'offline'
  | 'installing'
  | 'updating'
  | 'backing_up'
  | 'restoring'
  | 'degraded'
  | 'failed'
  | 'unknown';

export type GameInstanceHealth = 'healthy' | 'degraded' | 'unavailable' | 'unknown';

export type GameUpdateStatus =
  | 'current'
  | 'available'
  | 'pinned'
  | 'checking'
  | 'unknown';

export type GameBackupStatus =
  | 'healthy'
  | 'stale'
  | 'failed'
  | 'unconfigured'
  | 'unknown';

export type GameInstanceFeature =
  | 'console'
  | 'players'
  | 'settings'
  | 'mods_plugins'
  | 'worlds_saves'
  | 'files'
  | 'networking'
  | 'backups'
  | 'automation'
  | 'logs'
  | 'performance'
  | 'advanced';

export interface GameInstanceSummary {
  id: string;
  name: string;
  game: string;
  software: string;
  version: string;
  status: GameInstanceStatus;
  health: GameInstanceHealth;
  playersOnline: number;
  playersMax: number | null;
  cpuPercent: number | null;
  memoryUsedBytes: number | null;
  memoryLimitBytes: number | null;
  uptimeSeconds: number | null;
  address: string | null;
  updateStatus: GameUpdateStatus;
  backupStatus: GameBackupStatus;
  warnings: string[];
  features: GameInstanceFeature[];
}
