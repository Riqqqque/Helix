export interface HealthSnapshot {
  status: string;
  version: string;
  stateDatabase: string;
  metricsDatabase: string;
  timestampUnixMs: number;
}

export interface SetupStatus {
  ownerExists: boolean;
  bootstrapAvailable: boolean;
  bootstrapExpiresAtUnixMs: number | null;
}

export interface AuthenticatedUser {
  id: string;
  loginName: string;
  displayName: string;
  capabilities: string[];
}

export interface AuthSession {
  user: AuthenticatedUser;
  csrfToken: string;
  expiresAtUnixMs: number;
}

export interface AuthenticatedUserResponse {
  user: AuthenticatedUser;
  expiresAtUnixMs: number;
}

export interface CsrfRotation {
  csrfToken: string;
}

export interface OwnerSetupInput {
  bootstrapToken: string;
  loginName: string;
  displayName: string;
  password: string;
}

export interface LoginInput {
  loginName: string;
  password: string;
}

export interface AccountUpdateInput {
  currentPassword: string;
  loginName: string;
  displayName: string;
  newPassword?: string;
}

export interface CpuSnapshot {
  usagePercent: number | null;
  logicalCores: number;
}

export interface MemorySnapshot {
  totalBytes: number;
  usedBytes: number;
  availableBytes: number;
}

export interface SwapSnapshot {
  totalBytes: number;
  usedBytes: number;
}

export type DiscoveryAvailability = 'available' | 'degraded' | 'unavailable';

export interface StorageMountSnapshot {
  name: string | null;
  fileSystem: string | null;
  mountPoint: string | null;
  totalBytes: bigint;
  availableBytes: bigint;
  usedBytes: bigint;
  readOnly: boolean;
  removable: boolean;
}

export interface StorageSnapshot {
  availability: DiscoveryAvailability;
  mounts: StorageMountSnapshot[];
  omittedMounts: number;
  omittedTextFields: number;
}

export interface NetworkAddressSnapshot {
  address: string;
  prefixLength: number;
}

export interface NetworkInterfaceSnapshot {
  name: string;
  addresses: NetworkAddressSnapshot[];
  totalReceivedBytes: bigint;
  totalTransmittedBytes: bigint;
  mtuBytes: bigint;
}

export interface NetworkSnapshot {
  availability: DiscoveryAvailability;
  interfaces: NetworkInterfaceSnapshot[];
  omittedInterfaces: number;
  omittedAddresses: number;
}

export interface SystemOverview {
  hostname: string | null;
  operatingSystem: string | null;
  architecture: string;
  kernelVersion: string | null;
  uptimeSeconds: number;
  cpu: CpuSnapshot;
  memory: MemorySnapshot;
  swap: SwapSnapshot;
  storage: StorageSnapshot;
  network: NetworkSnapshot;
  collectedAtUnixMs: number;
}
