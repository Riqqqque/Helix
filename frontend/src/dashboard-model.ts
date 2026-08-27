import type { HostInventory, ManagedServer } from './control-api';
import type { RefreshIntervalMs } from './dashboard-preferences';
import type { HostIntegration } from './host-api';
import type { SystemOverview } from './types';

export type LoadPhase = 'loading' | 'refreshing' | 'ready' | 'stale' | 'error';

export interface DashboardResource<T> {
  data: T | null;
  phase: LoadPhase;
  error: string | null;
}

export interface DashboardData {
  overview: DashboardResource<SystemOverview>;
  inventory: DashboardResource<HostInventory>;
  servers: DashboardResource<ManagedServer[]>;
  integration: DashboardResource<HostIntegration>;
  refresh: () => Promise<void>;
  isRefreshing: boolean;
  refreshIntervalMs: RefreshIntervalMs;
}
