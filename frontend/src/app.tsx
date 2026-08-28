import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { ApiError, getSystemOverview } from './api';
import {
  getHostInventory,
  getServers,
  type HostInventory,
  type ManagedServer,
} from './control-api';
import type { DashboardData, DashboardResource as Resource } from './dashboard-model';
import {
  applyDashboardColors,
  type PrimaryDashboardSectionId,
  type RefreshIntervalMs,
} from './dashboard-preferences';
import { InlineError, Metric, PageHead, ProgressBar, toneForPercent } from './dashboard-ui';
import { FileManagerRoute, preloadFileManager } from './file-manager-route';
import { calculatePercent, formatBytes, formatDuration, formatPercent } from './format';
import { HomeRoute, preloadHomeRoute } from './home-route';
import { getHostIntegration, type HostIntegration } from './host-api';
import { HooksRoute, preloadHooksRoute } from './hooks-route';
import { Icon, type IconName } from './icons';
import { InfoTip } from './info-tip';
import {
  HostUpdatesRoute,
  NetworkOperationsRoute,
  preloadHostUpdatesRoute,
  preloadNetworkOperationsRoute,
} from './infrastructure-routes';
import { dashboardSectionForHash, type DashboardSectionId } from './navigation';
import { preloadServersRoute, ServersRoute } from './servers-route';
import { preloadSettingsRoute, SettingsRoute } from './settings-route';
import { preloadTerminalRoute, TerminalRoute } from './terminal-route';
import type { StorageAnalysisMode } from './storage-analysis-api';
import {
  applyThemePreference,
  readThemePreference,
  saveThemePreference,
  type ThemePreference,
} from './theme';
import type { AuthenticatedUser, SystemOverview } from './types';
import { useDashboardPreferences } from './use-dashboard-preferences';

const EMPTY_RESOURCE = { data: null, phase: 'loading', error: null } as const;
const navigation: ReadonlyArray<{
  id: DashboardSectionId;
  label: string;
  description: string;
  icon: IconName;
}> = [
  { id: 'overview', label: 'Overview', description: 'Live host summary', icon: 'overview' },
  { id: 'home', label: 'Home', description: 'Custom dashboard', icon: 'home' },
  { id: 'storage', label: 'Storage', description: 'Disks and files', icon: 'storage' },
  { id: 'network', label: 'Network', description: 'Interfaces and ports', icon: 'network' },
  { id: 'host', label: 'Host', description: 'Services and processes', icon: 'host' },
  { id: 'terminal', label: 'Terminal', description: 'Direct Linux shell', icon: 'terminal' },
  { id: 'servers', label: 'Servers', description: 'Game server instances', icon: 'servers' },
  { id: 'hooks', label: 'Hooks', description: 'Connected services', icon: 'hooks' },
  { id: 'settings', label: 'Settings', description: 'Dashboard and account', icon: 'settings' },
];

function preloadForSection(section: DashboardSectionId): (() => void) | undefined {
  if (section === 'storage') return preloadFileManager;
  if (section === 'home') return preloadHomeRoute;
  if (section === 'network') return preloadNetworkOperationsRoute;
  if (section === 'host') return preloadHostUpdatesRoute;
  if (section === 'terminal') return preloadTerminalRoute;
  if (section === 'servers') return preloadServersRoute;
  if (section === 'hooks') return preloadHooksRoute;
  if (section === 'settings') return preloadSettingsRoute;
  return undefined;
}

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : 'Helix could not complete that request.';
}

function isSessionError(error: unknown): boolean {
  return error instanceof ApiError && (error.status === 401 || error.code === 'csrf_rejected');
}

function useActiveSection(): DashboardSectionId {
  const initial = typeof window === 'undefined' ? '' : window.location.hash;
  const [section, setSection] = useState(() => dashboardSectionForHash(initial));
  useEffect(() => {
    const update = (): void => setSection(dashboardSectionForHash(window.location.hash));
    window.addEventListener('hashchange', update);
    return () => window.removeEventListener('hashchange', update);
  }, []);
  return section;
}

function useDashboardData(
  csrfToken: string,
  onSessionExpired: () => void,
  refreshIntervalMs: RefreshIntervalMs,
): DashboardData {
  const [overview, setOverview] = useState<Resource<SystemOverview>>(EMPTY_RESOURCE);
  const [inventory, setInventory] = useState<Resource<HostInventory>>(EMPTY_RESOURCE);
  const [servers, setServers] = useState<Resource<ManagedServer[]>>(EMPTY_RESOURCE);
  const [integration, setIntegration] = useState<Resource<HostIntegration>>(EMPTY_RESOURCE);
  const liveInFlight = useRef(false);
  const integrationGeneration = useRef(0);
  const mounted = useRef(true);
  const liveController = useRef<AbortController | null>(null);
  const integrationController = useRef<AbortController | null>(null);

  const refreshLive = useCallback(async (): Promise<void> => {
    if (liveInFlight.current) return;
    liveInFlight.current = true;
    liveController.current?.abort();
    const nextController = new AbortController();
    liveController.current = nextController;
    const markRefreshing = <T,>(current: Resource<T>): Resource<T> => ({
      ...current,
      phase: current.data === null ? 'loading' : 'refreshing',
      error: null,
    });
    setOverview(markRefreshing);
    setInventory(markRefreshing);
    setServers(markRefreshing);

    const results = await Promise.allSettled([
      getSystemOverview(csrfToken, nextController.signal),
      getHostInventory(csrfToken, nextController.signal),
      getServers(csrfToken, nextController.signal),
    ]);
    if (!mounted.current || nextController.signal.aborted) {
      liveInFlight.current = false;
      return;
    }
    if (results.some((result) => result.status === 'rejected' && isSessionError(result.reason))) {
      liveInFlight.current = false;
      onSessionExpired();
      return;
    }
    const apply = <T,>(
      result: PromiseSettledResult<T>,
      setter: (update: (current: Resource<T>) => Resource<T>) => void,
    ): void => {
      if (result.status === 'fulfilled') {
        setter(() => ({ data: result.value, phase: 'ready', error: null }));
      } else {
        setter((current) => ({
          data: current.data,
          phase: current.data === null ? 'error' : 'stale',
          error: describeError(result.reason),
        }));
      }
    };
    apply(results[0], setOverview);
    apply(results[1], setInventory);
    apply(results[2], setServers);
    liveInFlight.current = false;
  }, [csrfToken, onSessionExpired]);

  const refreshIntegration = useCallback(async (): Promise<void> => {
    integrationController.current?.abort();
    const generation = integrationGeneration.current + 1;
    integrationGeneration.current = generation;
    const nextController = new AbortController();
    integrationController.current = nextController;
    setIntegration((current) => ({
      ...current,
      phase: current.data === null ? 'loading' : 'refreshing',
      error: null,
    }));
    try {
      const next = await getHostIntegration(csrfToken, nextController.signal);
      if (!mounted.current || nextController.signal.aborted || integrationGeneration.current !== generation) return;
      setIntegration({ data: next, phase: 'ready', error: null });
    } catch (error) {
      if (!mounted.current || nextController.signal.aborted || integrationGeneration.current !== generation) return;
      if (isSessionError(error)) onSessionExpired();
      else {
        setIntegration((current) => ({
          data: current.data,
          phase: current.data === null ? 'error' : 'stale',
          error: describeError(error),
        }));
      }
    }
  }, [csrfToken, onSessionExpired]);

  const refresh = useCallback(async (): Promise<void> => {
    await Promise.all([refreshLive(), refreshIntegration()]);
  }, [refreshIntegration, refreshLive]);

  useEffect(() => {
    mounted.current = true;
    void refreshLive();
    const refreshWhenVisible = (): void => {
      if (document.visibilityState !== 'hidden') void refreshLive();
    };
    const timer = window.setInterval(refreshWhenVisible, refreshIntervalMs);
    document.addEventListener('visibilitychange', refreshWhenVisible);
    return () => {
      liveController.current?.abort();
      window.clearInterval(timer);
      document.removeEventListener('visibilitychange', refreshWhenVisible);
    };
  }, [refreshIntervalMs, refreshLive]);

  useEffect(() => {
    void refreshIntegration();
    const refreshWhenVisible = (): void => {
      if (document.visibilityState !== 'hidden') void refreshIntegration();
    };
    const timer = window.setInterval(refreshWhenVisible, 30_000);
    document.addEventListener('visibilitychange', refreshWhenVisible);
    return () => {
      integrationController.current?.abort();
      window.clearInterval(timer);
      document.removeEventListener('visibilitychange', refreshWhenVisible);
    };
  }, [refreshIntegration]);

  useEffect(() => () => {
    mounted.current = false;
    liveController.current?.abort();
    integrationController.current?.abort();
  }, []);

  return {
    overview,
    inventory,
    servers,
    integration,
    refresh,
    isRefreshing: [overview, inventory, servers, integration].some((resource) =>
      resource.phase === 'loading' || resource.phase === 'refreshing'),
    refreshIntervalMs,
  };
}

function HelixBrand() {
  return (
    <div class="helix-brand" aria-label="Helix">
      <span class="helix-mark" aria-hidden="true"><span /><span /></span>
      <span>HELIX</span>
    </div>
  );
}

function Sidebar({
  active,
  order,
  onOrderChange,
}: {
  active: DashboardSectionId;
  order: readonly PrimaryDashboardSectionId[];
  onOrderChange: (order: PrimaryDashboardSectionId[]) => void;
}) {
  const [arranging, setArranging] = useState(false);
  return (
    <aside class="sidebar">
      <HelixBrand />
      <nav class="sidebar-nav" aria-label="Dashboard">
        <div class="sidebar-nav__head"><span>Pages</span><button type="button" class={arranging ? 'is-active' : ''} aria-pressed={arranging} onClick={() => setArranging((value) => !value)}><Icon name={arranging ? 'check' : 'edit'} size={13} /><span>{arranging ? 'Done' : 'Arrange'}</span></button></div>
        <div class="sidebar-nav__primary">
          {order.map((section, index) => {
            const item = navigation.find((entry) => entry.id === section)!;
            return (
              <div class="nav-item-wrap" key={item.id}>
                <a href={`#${item.id}`} class={`nav-item${active === item.id ? ' is-active' : ''}`} aria-current={active === item.id ? 'page' : undefined} title={item.description} onPointerEnter={preloadForSection(item.id)} onFocus={preloadForSection(item.id)}>
                  <Icon name={item.icon} size={19} /><span>{item.label}</span>
                </a>
                {arranging && <div class="nav-item-order"><button type="button" disabled={index === 0} onClick={() => { const next = [...order]; [next[index - 1], next[index]] = [next[index]!, next[index - 1]!]; onOrderChange(next); }} aria-label={`Move ${item.label} up`}><Icon name="chevron" size={12} class="icon--up" /></button><button type="button" disabled={index === order.length - 1} onClick={() => { const next = [...order]; [next[index], next[index + 1]] = [next[index + 1]!, next[index]!]; onOrderChange(next); }} aria-label={`Move ${item.label} down`}><Icon name="chevron" size={12} class="icon--down" /></button></div>}
              </div>
            );
          })}
        </div>
        <a href="#settings" class={`nav-item nav-item--settings${active === 'settings' ? ' is-active' : ''}`} aria-current={active === 'settings' ? 'page' : undefined} title="Dashboard and account settings" onPointerEnter={preloadForSection('settings')} onFocus={preloadForSection('settings')}><Icon name="settings" size={19} /><span>Settings</span></a>
      </nav>
      <div class="sidebar-foot">
        <span class="status-dot status-dot--good" />
        <span>Connected</span>
      </div>
    </aside>
  );
}

function MobileNav({ active, order }: { active: DashboardSectionId; order: readonly PrimaryDashboardSectionId[] }) {
  const items = [...order, 'settings' as const].map((id) => navigation.find((entry) => entry.id === id)!);
  return (
    <nav class="mobile-nav" aria-label="Dashboard">
      {items.map((item) => (
        <a
          key={item.id}
          href={`#${item.id}`}
          class={active === item.id ? 'is-active' : ''}
          aria-current={active === item.id ? 'page' : undefined}
          onPointerEnter={preloadForSection(item.id)}
          onFocus={preloadForSection(item.id)}
        >
          <Icon name={item.icon} size={18} />
          <span>{item.label}</span>
        </a>
      ))}
    </nav>
  );
}

function ThemeMenu({ theme, onChange }: { theme: ThemePreference; onChange: (theme: ThemePreference) => void }) {
  const [open, setOpen] = useState(false);
  const setPreference = (next: ThemePreference): void => {
    onChange(next);
    setOpen(false);
  };
  const labelForTheme = (option: ThemePreference): string => {
    if (option === 'system') return 'Use device setting';
    if (option === 'oled') return 'OLED';
    return option[0]!.toUpperCase() + option.slice(1);
  };
  return (
    <div class="menu-wrap">
      <button class="icon-button" type="button" onClick={() => setOpen(!open)} aria-label="Change theme" aria-expanded={open}>
        <Icon name={theme === 'light' ? 'sun' : 'moon'} />
      </button>
      {open && (
        <div class="pop-menu pop-menu--theme">
          <strong>Theme</strong>
          {(['system', 'midnight', 'oled', 'light'] as const).map((option) => (
            <button type="button" onClick={() => setPreference(option)}>
              <span>{labelForTheme(option)}</span>
              {theme === option && <Icon name="check" size={15} />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function AccountMenu({ user, onLogout }: { user: AuthenticatedUser; onLogout: () => Promise<void> }) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  return (
    <div class="menu-wrap">
      <button class="account-button" type="button" onClick={() => setOpen(!open)} aria-expanded={open}>
        <span class="account-name">{user.displayName}</span>
        <Icon name="chevron" size={14} />
      </button>
      {open && (
        <div class="pop-menu pop-menu--account">
          <div class="account-detail">
            <strong>{user.displayName}</strong>
            <span>@{user.loginName}</span>
          </div>
          <button
            type="button"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              void onLogout().finally(() => setBusy(false));
            }}
          >
            Sign out
          </button>
        </div>
      )}
    </div>
  );
}

function Topbar({
  section,
  hostname,
  refreshedAt,
  user,
  onRefresh,
  onLogout,
  theme,
  onThemeChange,
}: {
  section: DashboardSectionId;
  hostname: string;
  refreshedAt: number | null;
  user: AuthenticatedUser;
  onRefresh: () => Promise<void>;
  onLogout: () => Promise<void>;
  theme: ThemePreference;
  onThemeChange: (theme: ThemePreference) => void;
}) {
  const item = navigation.find((entry) => entry.id === section) ?? navigation[0]!;
  const [manualRefresh, setManualRefresh] = useState(false);
  const refreshNow = async (): Promise<void> => {
    if (manualRefresh) return;
    setManualRefresh(true);
    try {
      await onRefresh();
    } finally {
      setManualRefresh(false);
    }
  };
  return (
    <header class="topbar">
      <div class="topbar-title">
        <span>{hostname}</span>
        <Icon name="chevron" size={13} />
        <strong>{item.label}</strong>
      </div>
      <div class="topbar-actions">
        {refreshedAt !== null && <span class="last-update">Updated {new Date(refreshedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</span>}
        <button class="icon-button" type="button" disabled={manualRefresh} onClick={() => void refreshNow()} aria-label="Refresh dashboard" aria-busy={manualRefresh}>
          <Icon name="refresh" />
        </button>
        <ThemeMenu theme={theme} onChange={onThemeChange} />
        <AccountMenu user={user} onLogout={onLogout} />
      </div>
    </header>
  );
}

function OverviewPage({ data }: { data: DashboardData }) {
  const { overview, inventory, servers, integration } = data;
  const memoryPercent = overview.data === null
    ? 0
    : calculatePercent(overview.data.memory.usedBytes, overview.data.memory.totalBytes) ?? 0;
  const criticalMounts = inventory.data?.mounts.filter((mount) => mount.usePercent >= 90) ?? [];
  const online = servers.data?.filter((server) => server.status === 'online').length ?? 0;
  const helixContainers = integration.data === null
    ? []
    : [integration.data.resources.containers.dashboard, integration.data.resources.containers.gateway].filter((value) => value !== null);
  const helixCpuValues = helixContainers.flatMap((value) => value.cpuPercent === null ? [] : [value.cpuPercent]);
  const helixMemoryValues = helixContainers.flatMap((value) => value.memoryUsedBytes === null ? [] : [value.memoryUsedBytes]);
  if (integration.data?.resources.broker.rssBytes !== null && integration.data?.resources.broker.rssBytes !== undefined) {
    helixMemoryValues.push(integration.data.resources.broker.rssBytes);
  }
  const helixCpu = helixCpuValues.length === 0 ? null : helixCpuValues.reduce((sum, value) => sum + value, 0);
  const helixMemory = helixMemoryValues.length === 0 ? null : helixMemoryValues.reduce((sum, value) => sum + value, 0);
  const helixErrors = integration.data?.errors ?? [];
  return (
    <div class="page page--overview">
      <PageHead title="Overview" detail={overview.data?.hostname
        ? `${overview.data.hostname} — host, storage, and game workloads in one place.`
        : 'The host, storage, and game workloads in one place.'} />
      <InlineError message={overview.error ?? inventory.error ?? servers.error ?? integration.error} />
      {criticalMounts.map((mount) => (
        <a class="capacity-alert" href="#storage" key={mount.target}>
          <Icon name="warning" />
          <div>
            <strong>{mount.target} is {formatPercent(mount.usePercent)} full</strong>
            <span>{formatBytes(mount.availableBytes)} remains on {mount.source}. Open Storage to make room.</span>
          </div>
          <Icon name="chevron" />
        </a>
      ))}
      <section class="metrics-strip" aria-label="Host metrics">
        <Metric icon="cpu" label="Processor" value={overview.data?.cpu.usagePercent === null || overview.data === null ? '—' : formatPercent(overview.data.cpu.usagePercent)} detail={`${overview.data?.cpu.logicalCores ?? '—'} logical cores`} percent={overview.data?.cpu.usagePercent ?? undefined} help="The share of this host’s CPU currently busy across all logical cores." />
        <Metric icon="memory" label="Memory" value={overview.data === null ? '—' : `${formatBytes(overview.data.memory.usedBytes)} / ${formatBytes(overview.data.memory.totalBytes)}`} detail={overview.data === null ? 'Loading' : `${formatBytes(overview.data.memory.availableBytes)} available`} percent={overview.data === null ? undefined : memoryPercent} help="RAM currently in use on the host. This includes the operating system, Helix, and every workload." />
        <Metric icon="servers" label="Game servers" value={servers.data === null ? '—' : `${online} online`} detail={servers.data === null ? 'Loading' : `${servers.data.filter((server) => server.manager === 'helix').length} Helix · ${servers.data.filter((server) => server.manager === 'amp_import').length} imported`} help="Helix-managed servers plus read-only compatibility inventory imported from AMP." />
        <Metric icon="activity" label="Uptime" value={overview.data === null ? '—' : formatDuration(overview.data.uptimeSeconds)} detail={inventory.data === null ? 'Loading' : `Load ${inventory.data.loadAverage.map((value) => value.toFixed(2)).join(' · ')}`} help="Time since the Linux host last booted. Load shows runnable work over 1, 5, and 15 minutes." />
      </section>
      <div class="overview-grid">
        <section class="surface overview-helix">
          <div class="section-title"><div><h2>Helix footprint <InfoTip text="These figures cover the Helix dashboard and gateway containers plus the privileged broker’s RAM. Game servers are deliberately excluded." /></h2><p>Dashboard runtime only · game servers excluded</p></div><span class={`state-label state-label--${integration.data?.availability === 'ready' && helixErrors.length === 0 ? 'good' : integration.data === null ? 'idle' : 'warning'}`}>{integration.data === null ? 'Loading' : helixErrors.length === 0 && integration.data.availability === 'ready' ? 'Healthy' : `${helixErrors.length} issue${helixErrors.length === 1 ? '' : 's'}`}</span></div>
          <div class="helix-footprint-stats">
            <div><span>Container CPU</span><strong>{helixCpu === null ? '—' : formatPercent(helixCpu)}</strong><small>Dashboard + gateway</small></div>
            <div><span>Runtime memory</span><strong>{helixMemory === null ? '—' : formatBytes(helixMemory)}</strong><small>Containers + broker RSS</small></div>
            <div><span>Runtime errors</span><strong>{integration.data === null ? '—' : helixErrors.length}</strong><small>{helixErrors.length === 0 ? 'No integration errors reported' : helixErrors[0]?.message}</small></div>
          </div>
        </section>
        <section class="surface overview-storage">
          <div class="section-title"><div><h2>Storage</h2><p>Mounted capacity across this host</p></div><a href="#storage">Open files <Icon name="chevron" size={14} /></a></div>
          <div class="mount-summary-list">
            {(inventory.data?.mounts ?? []).filter((mount) => mount.sizeBytes >= 10_000_000_000).slice(0, 5).map((mount) => (
              <div class="mount-summary" key={mount.target}><div class="mount-summary-top"><strong>{mount.target}</strong><span>{formatBytes(mount.usedBytes)} of {formatBytes(mount.sizeBytes)}</span></div><ProgressBar value={mount.usePercent} tone={toneForPercent(mount.usePercent)} /><small>{mount.source} · {formatBytes(mount.availableBytes)} free</small></div>
            ))}
          </div>
        </section>
        <section class="surface overview-servers">
          <div class="section-title"><div><h2>Servers</h2><p>Helix and imported workloads</p></div><a href="#servers">Manage <Icon name="chevron" size={14} /></a></div>
          <div class="compact-server-list">
            {(servers.data ?? []).slice().sort((a, b) => Number(b.status === 'online') - Number(a.status === 'online')).slice(0, 6).map((server) => (
              <div key={server.id}><span class={`status-dot status-dot--${server.status === 'online' ? 'good' : 'idle'}`} /><strong>{server.name}</strong><span>{server.status === 'online' ? `${server.playersOnline}/${server.maxPlayers} players` : 'Offline'}</span><small>{server.software} {server.version}</small></div>
            ))}
          </div>
        </section>
        <section class="surface overview-services">
          <div class="section-title"><div><h2>Services</h2><p>Systemd units Helix can see</p></div><a href="#host">Inspect <Icon name="chevron" size={14} /></a></div>
          <div class="service-pills">{(inventory.data?.services ?? []).slice(0, 8).map((service) => <span key={service.unit}><i class={`status-dot status-dot--${service.active === 'active' ? 'good' : 'idle'}`} />{service.unit.replace('.service', '')}</span>)}</div>
        </section>
        <section class="surface overview-network">
          <div class="section-title"><div><h2>Network</h2><p>Physical and virtual interfaces</p></div><a href="#network">Details <Icon name="chevron" size={14} /></a></div>
          <div class="interface-summary">{(inventory.data?.interfaces ?? []).filter((item) => item.name !== 'lo').slice(0, 4).map((item) => <div key={item.name}><span class={`status-dot status-dot--${item.state.toLowerCase() === 'up' ? 'good' : 'idle'}`} /><strong>{item.name}</strong><span>{item.addresses.find((address) => address.family === 'inet')?.address ?? 'No IPv4 address'}</span></div>)}</div>
        </section>
      </div>
    </div>
  );
}

export function DiskMap({
  inventory,
  onBrowse,
  onAnalyze,
}: {
  inventory: HostInventory | null;
  onBrowse: (path: string) => void;
  onAnalyze: (path: string) => void;
}) {
  const disks = inventory?.disks.filter((disk) => disk.deviceType === 'disk') ?? [];
  return (
    <section class="disk-map">
      {disks.map((disk) => {
        const mounts = inventory?.mounts.filter((mount) => disk.path !== null && mount.source.startsWith(disk.path)) ?? [];
        const primary = mounts.sort((a, b) => b.sizeBytes - a.sizeBytes)[0];
        return (
          <article class={'disk-tile' + (primary === undefined ? ' is-unmounted' : '')} key={disk.name}>
            <div class="disk-tile-icon"><Icon name="storage" size={22} /></div>
            <div class="disk-tile-copy"><span>{disk.model?.trim() || disk.name}</span><strong>{formatBytes(disk.sizeBytes)}</strong><small>{primary === undefined ? 'Not mounted' : `${primary.target} · ${formatBytes(primary.availableBytes)} free`}</small></div>
            {primary !== undefined && <div class="disk-tile-usage"><span>{formatPercent(primary.usePercent)}</span><ProgressBar value={primary.usePercent} tone={toneForPercent(primary.usePercent)} /></div>}
            {primary !== undefined && <div class="disk-tile-actions"><button class="button button--quiet" type="button" onClick={() => onBrowse(primary.target)}><Icon name="folder" size={14} />Browse files</button><button class="button button--primary" type="button" onClick={() => onAnalyze(primary.target)}><Icon name="search" size={14} />Analyze space</button></div>}
          </article>
        );
      })}
    </section>
  );
}

function StoragePage({ data, csrfToken, onSessionExpired }: { data: DashboardData; csrfToken: string; onSessionExpired: () => void }) {
  const [browsePath, setBrowsePath] = useState('/');
  const [analysis, setAnalysis] = useState<{ path: string; mode: StorageAnalysisMode } | null>(null);
  const openDriveAnalysis = (path: string): void => setAnalysis({ path, mode: 'thorough' });
  return <div class="page page--storage"><PageHead title="Storage" detail="See what is using each drive, then manage files safely." /><InlineError message={data.inventory.error} /><section class="storage-space-intro"><div><span class="eyebrow">SPACE ANALYZER</span><h2>Find what’s using your drive</h2><p>Choose <strong>Analyze space</strong> on any mounted drive. Helix reads filesystem metadata in the background and ranks the files and folder trees consuming the most disk space.</p></div><div class="storage-space-intro__facts"><span><Icon name="activity" size={14} />Low-impact scan</span><span><Icon name="storage" size={14} />Allocated disk usage</span><span><Icon name="check" size={14} />Read-only until you choose trash</span></div></section><DiskMap inventory={data.inventory.data} onBrowse={setBrowsePath} onAnalyze={openDriveAnalysis} /><div class="section-title section-title--spaced"><div><h2>Files <InfoTip text="Helix can browse the host but only changes files inside storage roots allowed by the privileged broker. Delete actions move items into a recoverable .helix-trash folder." /></h2><p>Browse the whole host. Changes are limited to configured storage roots.</p></div><button class="button button--quiet" type="button" onClick={() => setAnalysis({ path: browsePath, mode: 'quick' })}><Icon name="search" size={15} />Analyze current folder</button></div><FileManagerRoute csrfToken={csrfToken} onSessionExpired={onSessionExpired} initialPath={browsePath} analysis={analysis} onAnalysisClose={() => setAnalysis(null)} /></div>;
}

function NetworkPage({ data, csrfToken, canManageFirewall, onSessionExpired }: { data: DashboardData; csrfToken: string; canManageFirewall: boolean; onSessionExpired: () => void }) {
  const inventory = data.inventory.data;
  return (
    <div class="page page--network">
      <PageHead title="Network" detail="Interfaces, routing, listeners, and game port allocation." />
      <InlineError message={data.inventory.error} />
      <div class="section-title section-title--plain"><div><h2>Interfaces <InfoTip text="A network interface is a physical, virtual, bridge, or tunnel connection. Its addresses show how this host can be reached on each network." /></h2><p>Physical, virtual, container, and tunnel connections</p></div></div>
      <section class="interface-grid">
        {(inventory?.interfaces ?? []).map((item) => (
          <div class="interface-panel" key={item.name}>
            <header><div><span class={`status-dot status-dot--${item.state.toLowerCase() === 'up' ? 'good' : 'idle'}`} /><strong>{item.name}</strong></div><span>{item.state}</span></header>
            <div class="interface-addresses">{item.addresses.length === 0 ? <span>No addresses</span> : item.addresses.map((address) => <code key={`${address.family}-${address.address}`}>{address.address}/{address.prefixLength}</code>)}</div>
            <dl><div><dt>Received</dt><dd>{formatBytes(item.receivedBytes)}</dd></div><div><dt>Sent</dt><dd>{formatBytes(item.transmittedBytes)}</dd></div><div><dt>MTU</dt><dd>{item.mtu}</dd></div><div><dt>Errors</dt><dd>{item.receivedErrors + item.transmittedErrors}</dd></div></dl>
            <small>{item.mac ?? 'No hardware address'}</small>
          </div>
        ))}
      </section>
      <section class="surface infrastructure-section">
        <div class="section-title"><div><h2>Routes <InfoTip text="Routes decide which interface and gateway Linux uses for each destination. The default route handles traffic that has no more specific match." /></h2><p>Kernel routing table</p></div></div>
        <div class="table-scroll"><table class="data-table"><thead><tr><th>Destination</th><th>Gateway</th><th>Interface</th><th>Metric</th></tr></thead><tbody>{(inventory?.routes ?? []).map((route, index) => <tr key={`${route.destination}-${route.gateway}-${index}`}><td><code>{route.destination}</code></td><td>{route.gateway ?? 'Direct'}</td><td>{route.interface ?? '—'}</td><td>{route.metric ?? '—'}</td></tr>)}</tbody></table></div>
      </section>
      <NetworkOperationsRoute csrfToken={csrfToken} canManageFirewall={canManageFirewall} onSessionExpired={onSessionExpired} />
    </div>
  );
}

function HostPage({ data, csrfToken, onSessionExpired }: { data: DashboardData; csrfToken: string; onSessionExpired: () => void }) {
  const overview = data.overview.data;
  const inventory = data.inventory.data;
  return (
    <div class="page page--host">
      <PageHead title="Host" detail="Operating system, services, and the processes using this machine." /><InlineError message={data.overview.error ?? data.inventory.error} />
      <section class="host-facts"><div><span>Hostname</span><strong>{overview?.hostname ?? '—'}</strong></div><div><span>Operating system</span><strong>{overview?.operatingSystem ?? '—'}</strong></div><div><span>Kernel</span><strong>{overview?.kernelVersion ?? '—'}</strong></div><div><span>Architecture</span><strong>{overview?.architecture ?? '—'}</strong></div><div><span>Uptime</span><strong>{overview === null ? '—' : formatDuration(overview.uptimeSeconds)}</strong></div><div><span>Load average</span><strong>{inventory?.loadAverage.map((value) => value.toFixed(2)).join(' / ') ?? '—'}</strong></div></section>
      <div class="host-columns">
        <section class="surface host-table-panel"><div class="section-title"><div><h2>Services <InfoTip text="Services are background programs managed by systemd. Active means systemd currently considers the unit running; failed or inactive units may need attention depending on their purpose." /></h2><p>Systemd service state</p></div></div><div class="host-table-scroll"><table class="data-table services-table"><thead><tr><th>Unit</th><th>State</th><th>Description</th></tr></thead><tbody>{(inventory?.services ?? []).map((service) => <tr key={service.unit}><td><strong>{service.unit}</strong></td><td><span class={`state-label state-label--${service.active === 'active' ? 'good' : 'idle'}`}>{service.active}</span></td><td>{service.description}</td></tr>)}</tbody></table></div></section>
        <section class="surface host-table-panel"><div class="section-title"><div><h2>Top processes <InfoTip text="A process is a running program. This list is sampled and sorted by current CPU use so a busy or runaway workload is easier to spot." /></h2><p>Sorted by current CPU use</p></div></div><div class="host-table-scroll"><table class="data-table processes-table"><thead><tr><th>Process</th><th>CPU</th><th>Memory</th><th>Uptime</th></tr></thead><tbody>{(inventory?.processes ?? []).map((process) => <tr key={process.pid}><td><strong>{process.name}</strong><small>PID {process.pid} · {process.user}</small></td><td>{formatPercent(process.cpuPercent)}</td><td>{formatBytes(process.residentBytes)}</td><td>{formatDuration(process.uptimeSeconds)}</td></tr>)}</tbody></table></div></section>
      </div>
      <HostUpdatesRoute csrfToken={csrfToken} onSessionExpired={onSessionExpired} />
    </div>
  );
}

export interface DashboardProps {
  user: AuthenticatedUser;
  csrfToken: string;
  onSessionExpired: () => void;
  onAccountUpdated: () => void;
  onLogout: () => Promise<void>;
}

export function Dashboard({ user, csrfToken, onSessionExpired, onAccountUpdated, onLogout }: DashboardProps) {
  const active = useActiveSection();
  const [theme, setTheme] = useState<ThemePreference>(readThemePreference);
  const dashboardPreferences = useDashboardPreferences(csrfToken, onSessionExpired);
  const { navigationOrder, metricsRefreshMs: refreshIntervalMs } = dashboardPreferences;
  const data = useDashboardData(csrfToken, onSessionExpired, refreshIntervalMs);
  useEffect(() => {
    applyDashboardColors(dashboardPreferences.colors);
  }, [dashboardPreferences.colors]);
  const refreshedAt = Math.max(data.overview.data?.collectedAtUnixMs ?? 0, data.inventory.data?.collectedAtUnixMs ?? 0, data.integration.data?.collectedAtUnixMs ?? 0) || null;
  const hostname = data.overview.data?.hostname ?? 'Server';

  useEffect(() => {
    saveThemePreference(theme);
    const colorPreference = window.matchMedia('(prefers-color-scheme: light)');
    const apply = (): void => { applyThemePreference(theme, colorPreference.matches); };
    apply();
    if (theme === 'system') colorPreference.addEventListener('change', apply);
    return () => colorPreference.removeEventListener('change', apply);
  }, [theme]);

  const changeNavigationOrder = (next: PrimaryDashboardSectionId[]): void => {
    dashboardPreferences.setNavigationOrder(next);
  };
  const changeRefreshInterval = (next: RefreshIntervalMs): void => {
    dashboardPreferences.setMetricsRefreshMs(next);
  };

  return <><a class="skip-link" href="#main-content">Skip to content</a><div class="dashboard-shell"><Sidebar active={active} order={navigationOrder} onOrderChange={changeNavigationOrder} /><div class="dashboard-workspace"><Topbar section={active} hostname={hostname} refreshedAt={refreshedAt} user={user} onRefresh={data.refresh} onLogout={onLogout} theme={theme} onThemeChange={setTheme} /><MobileNav active={active} order={navigationOrder} /><main id="main-content" tabIndex={-1}>{active === 'overview' && <OverviewPage data={data} />}{active === 'home' && <HomeRoute overview={data.overview.data} inventory={data.inventory.data} servers={data.servers.data ?? []} displayName={user.displayName} templates={dashboardPreferences.homeTemplates} activeHomeId={dashboardPreferences.activeHomeId} syncStatus={dashboardPreferences.syncStatus} onHomeChange={dashboardPreferences.setHomeState} csrfToken={csrfToken} />}{active === 'storage' && <StoragePage data={data} csrfToken={csrfToken} onSessionExpired={onSessionExpired} />}{active === 'network' && <NetworkPage data={data} csrfToken={csrfToken} canManageFirewall={user.capabilities.includes('network.firewall.write')} onSessionExpired={onSessionExpired} />}{active === 'host' && <HostPage data={data} csrfToken={csrfToken} onSessionExpired={onSessionExpired} />}{active === 'terminal' && <TerminalRoute csrfToken={csrfToken} canOpen={user.capabilities.includes('terminal.open')} onSessionExpired={onSessionExpired} />}{active === 'servers' && <ServersRoute data={data} csrfToken={csrfToken} canManageServers={user.capabilities.includes('games.manage')} canManageBackups={user.capabilities.includes('games.backups.manage')} canManageNetwork={user.capabilities.includes('network.firewall.write')} onSessionExpired={onSessionExpired} />}{active === 'hooks' && <HooksRoute csrfToken={csrfToken} canManage={user.capabilities.includes('system.settings.write')} onSessionExpired={onSessionExpired} />}{active === 'settings' && <SettingsRoute user={user} csrfToken={csrfToken} theme={theme} refreshIntervalMs={refreshIntervalMs} navigationOrder={navigationOrder} colors={dashboardPreferences.colors} preferenceSyncStatus={dashboardPreferences.syncStatus} hostIntegration={data.integration} onThemeChange={setTheme} onRefreshIntervalChange={changeRefreshInterval} onNavigationOrderChange={changeNavigationOrder} onColorsChange={dashboardPreferences.setColors} onAccountUpdated={onAccountUpdated} onHostIntegrationRefresh={data.refresh} />}</main></div></div></>;
}
