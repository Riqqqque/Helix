import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { ApiError, getSystemOverview } from './api';
import { OperationError } from './operation-error';
import {
  getJob,
  getHostInventory,
  getServers,
  type HostInventory,
  type ManagedServer,
} from './control-api';
import {
  forgetResumableOperation,
  OPERATION_CONTINUITY_EVENT,
  readResumableOperations,
  updateResumableOperation,
  type ResumableOperation,
} from './operation-continuity';
import type { DashboardData, DashboardResource as Resource } from './dashboard-model';
import {
  applyDashboardColors,
  visibleDashboardSections,
  type PrimaryDashboardSectionId,
  type RefreshIntervalMs,
} from './dashboard-preferences';
import { PageHead } from './dashboard-ui';
import { preloadFileManager } from './file-manager-route';
import { formatBytes, formatPercent } from './format';
import { HomeRoute, preloadHomeRoute } from './home-route';
import { preloadDockerPanel } from './docker-panel-route';
import { OverviewRoute, preloadOverviewRoute } from './overview-route';
import { readHomeFocus, saveHomeFocus } from './home-layout';
import { getHostIntegration, type HostIntegration } from './host-api';
import { HooksRoute, preloadHooksRoute } from './hooks-route';
import { StrandsRoute, preloadStrandsRoute } from './strands-route';
import { GlobeRoute, preloadGlobeRoute } from './globe-route';
import { SecurityRoute, preloadSecurityRoute } from './security-route';
import { Icon, type IconName } from './icons';
import { DISMISSALS_CHANGED_EVENT, dismissNotice, isDismissed } from './dismissals';
import {
  preloadHostUpdatesRoute,
  preloadNetworkOperationsRoute,
} from './infrastructure-routes';
import { dashboardSectionForHash, type DashboardSectionId } from './navigation';
import type { NavArrangeApi } from './nav-arrange';
import { preloadServersRoute, ServersRoute } from './servers-route';
import { preloadSettingsRoute, SettingsRoute } from './settings-route';
import { preloadTerminalRoute, TerminalRoute } from './terminal-route';
import {
  HostPageRoute,
  NetworkPageRoute,
  preloadWorkspacePages,
  StoragePageRoute,
} from './workspace-pages-route';
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
  { id: 'security', label: 'Security', description: 'Host and Helix protections', icon: 'security' },
  { id: 'terminal', label: 'Terminal', description: 'Direct Linux shell', icon: 'terminal' },
  { id: 'servers', label: 'Servers', description: 'Game server instances', icon: 'servers' },
  { id: 'hooks', label: 'Hooks', description: 'Connected services', icon: 'hooks' },
  { id: 'strands', label: 'Strands', description: 'Installable extensions', icon: 'strands' },
  { id: 'globe', label: 'Globe', description: 'World map of connections', icon: 'globe' },
  { id: 'settings', label: 'Settings', description: 'Dashboard and account', icon: 'settings' },
];

function preloadForSection(section: DashboardSectionId, csrfToken: string): (() => void) | undefined {
  if (section === 'storage') return () => { preloadFileManager(); preloadWorkspacePages(); };
  if (section === 'home') return preloadHomeRoute;
  if (section === 'network') return () => { preloadNetworkOperationsRoute(); preloadWorkspacePages(); };
  if (section === 'host') return () => { preloadHostUpdatesRoute(); preloadDockerPanel(); preloadWorkspacePages(); };
  if (section === 'overview') return () => { preloadOverviewRoute(); preloadDockerPanel(); };
  if (section === 'terminal') return preloadTerminalRoute;
  if (section === 'servers') return preloadServersRoute;
  if (section === 'hooks') return () => preloadHooksRoute(csrfToken);
  if (section === 'strands') return preloadStrandsRoute;
  if (section === 'globe') return preloadGlobeRoute;
  if (section === 'security') return () => preloadSecurityRoute(csrfToken);
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
  hostIntegrationNeeded: boolean,
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
    await Promise.all([
      refreshLive(),
      hostIntegrationNeeded ? refreshIntegration() : Promise.resolve(),
    ]);
  }, [hostIntegrationNeeded, refreshIntegration, refreshLive]);

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
    if (!hostIntegrationNeeded) return;
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
  }, [hostIntegrationNeeded, refreshIntegration]);

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
      <span>HELIX</span>
    </div>
  );
}

function Sidebar({
  active,
  csrfToken,
  order,
  hiddenPages,
  serversEnabled,
  onOrderChange,
  onHiddenPagesChange,
  onServersEnabledChange,
}: {
  active: DashboardSectionId;
  csrfToken: string;
  order: readonly PrimaryDashboardSectionId[];
  hiddenPages: readonly PrimaryDashboardSectionId[];
  serversEnabled: boolean;
  onOrderChange: (order: PrimaryDashboardSectionId[]) => void;
  onHiddenPagesChange: (pages: PrimaryDashboardSectionId[]) => void;
  onServersEnabledChange: (enabled: boolean) => void;
}) {
  const [arranging, setArranging] = useState(false);
  const [arrange, setArrange] = useState<NavArrangeApi | null>(null);
  const visibleOrder = visibleDashboardSections(order, hiddenPages, serversEnabled);
  const loadArrange = (): void => {
    if (arrange !== null) return;
    void import('./nav-arrange').then((module) => setArrange(module));
  };
  return (
    <aside class="sidebar">
      <HelixBrand />
      <nav class="sidebar-nav" aria-label="Dashboard">
        <div class="sidebar-nav__head"><span>Pages</span><button type="button" class={arranging ? 'is-active' : ''} aria-pressed={arranging} onPointerEnter={loadArrange} onFocus={loadArrange} onClick={() => { loadArrange(); setArranging((value) => !value); }}><Icon name={arranging ? 'check' : 'edit'} size={13} /><span>{arranging ? 'Done' : 'Arrange'}</span></button></div>
        <div class="sidebar-nav__primary">
          {visibleOrder.map((section) => {
            const item = navigation.find((entry) => entry.id === section)!;
            const visibleIndex = visibleOrder.indexOf(section);
            return (
              <div class={`nav-item-wrap${arranging ? ' is-arranging' : ''}`} key={item.id}>
                <a href={`#${item.id}`} class={`nav-item${active === item.id ? ' is-active' : ''}`} aria-current={active === item.id ? 'page' : undefined} title={item.description} onPointerEnter={preloadForSection(item.id, csrfToken)} onFocus={preloadForSection(item.id, csrfToken)}>
                  <Icon name={item.icon} size={19} /><span>{item.label}</span>
                </a>
                {arranging && arrange !== null && (
                  <arrange.NavOrderButtons
                    label={item.label}
                    section={section}
                    visibleIndex={visibleIndex}
                    visibleCount={visibleOrder.length}
                    order={order}
                    hiddenPages={hiddenPages}
                    serversEnabled={serversEnabled}
                    onOrderChange={onOrderChange}
                    onHide={(page) => arrange.hideArrangedPage(hiddenPages, page, serversEnabled, order, active, onHiddenPagesChange, onServersEnabledChange)}
                  />
                )}
              </div>
            );
          })}
        </div>
        {arranging && arrange !== null && (
          <arrange.NavAddCatalog
            order={order}
            hiddenPages={hiddenPages}
            serversEnabled={serversEnabled}
            pages={navigation}
            onAdd={(page) => arrange.addArrangedPage(hiddenPages, page, onHiddenPagesChange, onServersEnabledChange)}
          />
        )}
        <a href="#settings" class={`nav-item nav-item--settings${active === 'settings' ? ' is-active' : ''}`} aria-current={active === 'settings' ? 'page' : undefined} title="Dashboard and account settings" onPointerEnter={preloadForSection('settings', csrfToken)} onFocus={preloadForSection('settings', csrfToken)}><Icon name="settings" size={19} /><span>Settings</span></a>
      </nav>
      <div class="sidebar-foot">
        <span class="status-dot status-dot--good" />
        <span>Connected</span>
        <a class="sidebar-help" href="https://github.com/Riqqqque/Helix/wiki" target="_blank" rel="noreferrer">Help</a>
      </div>
    </aside>
  );
}

function MobileNav({
  active,
  csrfToken,
  order,
  hiddenPages,
  serversEnabled,
}: {
  active: DashboardSectionId;
  csrfToken: string;
  order: readonly PrimaryDashboardSectionId[];
  hiddenPages: readonly PrimaryDashboardSectionId[];
  serversEnabled: boolean;
}) {
  const visible = visibleDashboardSections(order, hiddenPages, serversEnabled);
  const items = [...visible, 'settings' as const].map((id) => navigation.find((entry) => entry.id === id)!);
  return (
    <nav class="mobile-nav" aria-label="Dashboard">
      {items.map((item) => (
        <a
          key={item.id}
          href={`#${item.id}`}
          class={active === item.id ? 'is-active' : ''}
          aria-current={active === item.id ? 'page' : undefined}
          onPointerEnter={preloadForSection(item.id, csrfToken)}
          onFocus={preloadForSection(item.id, csrfToken)}
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

function NotificationsMenu({
  items,
}: {
  items: ReadonlyArray<{ id: string; title: string; body: string; href: string }>;
}) {
  const [open, setOpen] = useState(false);
  const [tick, setTick] = useState(0);
  const visible = items.filter((item) => !isDismissed(item.id));
  void tick;
  useEffect(() => {
    const refresh = (): void => setTick((value) => value + 1);
    window.addEventListener(DISMISSALS_CHANGED_EVENT, refresh);
    return () => window.removeEventListener(DISMISSALS_CHANGED_EVENT, refresh);
  }, []);
  return (
    <div class="notifications-wrap">
      <button
        class="icon-button"
        type="button"
        aria-expanded={open}
        aria-label={visible.length === 0 ? 'Notifications' : `${visible.length} notifications`}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon name="bell" />
      </button>
      {open && (
        <div class="notifications-menu" role="dialog" aria-label="Notifications">
          <h2>Notifications</h2>
          {visible.length === 0 ? <p>No notices right now.</p> : visible.map((item) => (
            <div class="notifications-item" key={item.id}>
              <div>
                <strong>{item.title}</strong>
                <span>{item.body}</span>
                <a href={item.href}>Open</a>
              </div>
              <button class="icon-button" type="button" aria-label={`Dismiss ${item.title}`} onClick={() => { dismissNotice(item.id); setTick((value) => value + 1); }}>
                <Icon name="close" size={14} />
              </button>
            </div>
          ))}
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
  notices,
  onRefresh,
  onLogout,
  theme,
  onThemeChange,
}: {
  section: DashboardSectionId;
  hostname: string;
  refreshedAt: number | null;
  user: AuthenticatedUser;
  notices: ReadonlyArray<{ id: string; title: string; body: string; href: string }>;
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
        <NotificationsMenu items={notices} />
        <button class="icon-button" type="button" disabled={manualRefresh} onClick={() => void refreshNow()} aria-label="Refresh dashboard" aria-busy={manualRefresh}>
          <Icon name="refresh" />
        </button>
        <ThemeMenu theme={theme} onChange={onThemeChange} />
        <AccountMenu user={user} onLogout={onLogout} />
      </div>
    </header>
  );
}

export interface DashboardProps {
  user: AuthenticatedUser;
  csrfToken: string;
  onSessionExpired: () => void;
  onAccountUpdated: () => void;
  onLogout: () => Promise<void>;
}

function ServersModuleDisabled({ onEnable }: { onEnable: () => void }) {
  return (
    <div class="page page--servers-disabled">
      <PageHead title="Servers" detail="This Helix does not show the game-server dashboard yet." />
      <section class="surface servers-disabled-card">
        <Icon name="servers" size={28} />
        <strong>Game servers are turned off</strong>
        <p>
          Existing Minecraft, V Rising, and imported servers keep running. Turn this on to create, start, stop, and manage them from Helix.
        </p>
        <button class="button button--primary" type="button" onClick={onEnable}>
          Enable Servers
        </button>
        <small>You can also do this later from Settings.</small>
      </section>
    </div>
  );
}

function OperationContinuity({
  csrfToken,
  onRefresh,
  onSessionExpired,
}: {
  csrfToken: string;
  onRefresh: () => Promise<void>;
  onSessionExpired: () => void;
}) {
  const [operations, setOperations] = useState<ResumableOperation[]>(readResumableOperations);
  const [trackingError, setTrackingError] = useState<string | null>(null);
  const completed = useRef(new Set<string>());

  useEffect(() => {
    const sync = (): void => setOperations(readResumableOperations());
    window.addEventListener(OPERATION_CONTINUITY_EVENT, sync);
    return () => window.removeEventListener(OPERATION_CONTINUITY_EVENT, sync);
  }, []);

  const activeIds = operations
    .filter((item) => item.status === 'queued' || item.status === 'running')
    .map((item) => item.id)
    .join(',');

  useEffect(() => {
    if (activeIds.length === 0) return;
    const controller = new AbortController();
    let inFlight = false;
    const poll = (): void => {
      if (inFlight) return;
      inFlight = true;
      const ids = activeIds.split(',');
      void Promise.allSettled(ids.map((id) => getJob(id, csrfToken, controller.signal))).then((results) => {
        if (controller.signal.aborted) return;
        let shouldRefresh = false;
        let firstError: string | null = null;
        for (const result of results) {
          if (result.status === 'fulfilled') {
            updateResumableOperation(result.value);
            if ((result.value.status === 'complete' || result.value.status === 'failed') && !completed.current.has(result.value.id)) {
              completed.current.add(result.value.id);
              shouldRefresh = true;
            }
          } else if (isSessionError(result.reason)) {
            onSessionExpired();
            return;
          } else if (firstError === null) {
            firstError = describeError(result.reason);
          }
        }
        setTrackingError(firstError);
        setOperations(readResumableOperations());
        if (shouldRefresh) void onRefresh();
      }).finally(() => { inFlight = false; });
    };
    const timer = window.setInterval(poll, 1_000);
    return () => {
      window.clearInterval(timer);
      controller.abort();
    };
  }, [activeIds, csrfToken, onRefresh, onSessionExpired]);

  if (operations.length === 0) return null;
  return (
    <aside class="operation-continuity" aria-label="Background operations" aria-live="polite">
      <div class="operation-continuity__head">
        <span><Icon name="performance" size={16} /><strong>Background activity</strong></span>
        <small>Safe to refresh</small>
      </div>
      {operations.map((operation) => (
        <div class={`operation-continuity__item operation-continuity__item--${operation.status}`} key={operation.id}>
          <div>
            <strong>{operation.label}</strong>
            <span>{operation.stage}</span>
            {(operation.status === 'queued' || operation.status === 'running') && (
              <progress max={100} value={operation.progressPercent}>{operation.progressPercent}%</progress>
            )}
            {operation.error !== null && <OperationError message={operation.error} />}
          </div>
          {(operation.status === 'complete' || operation.status === 'failed') && (
            <button class="icon-button" type="button" aria-label={`Dismiss ${operation.label}`} onClick={() => {
              forgetResumableOperation(operation.id);
              setOperations(readResumableOperations());
            }}><Icon name="close" size={14} /></button>
          )}
        </div>
      ))}
      {trackingError !== null && <small class="operation-continuity__error">Status check delayed: {trackingError} Helix will retry; the action was not repeated.</small>}
    </aside>
  );
}

export function Dashboard({ user, csrfToken, onSessionExpired, onAccountUpdated, onLogout }: DashboardProps) {
  const active = useActiveSection();
  const [theme, setTheme] = useState<ThemePreference>(readThemePreference);
  const dashboardPreferences = useDashboardPreferences(csrfToken, onSessionExpired);
  const { navigationOrder, hiddenPages, metricsRefreshMs: refreshIntervalMs, serversEnabled } = dashboardPreferences;
  const hostIntegrationNeeded = active === 'overview' || active === 'settings';
  const data = useDashboardData(csrfToken, onSessionExpired, refreshIntervalMs, hostIntegrationNeeded);
  useEffect(() => {
    preloadForSection(active, csrfToken)?.();
  }, [active, csrfToken]);
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

  const [homeFocus, setHomeFocus] = useState(readHomeFocus);
  const homeFocused = active === 'home' && homeFocus;
  const themeLabel = theme === 'system' ? 'System' : theme === 'midnight' ? 'Midnight' : theme === 'oled' ? 'OLED' : 'Light';
  const toggleHomeFocus = (): void => {
    setHomeFocus((current) => {
      const next = !current;
      saveHomeFocus(next);
      return next;
    });
  };

  const notices = (data.inventory.data?.mounts ?? [])
    .filter((mount) => mount.usePercent >= 90)
    .map((mount) => ({
      id: `capacity:${mount.target}`,
      title: `${mount.target} is ${formatPercent(mount.usePercent)} full`,
      body: `${formatBytes(mount.availableBytes)} remains on ${mount.source}.`,
      href: '#storage',
    }));

  return (
    <>
      <a class="skip-link" href="#main-content">Skip to content</a>
      <div class={`dashboard-shell${homeFocused ? ' is-home-focus' : ''}`}>
        <Sidebar active={active} csrfToken={csrfToken} order={navigationOrder} hiddenPages={hiddenPages} serversEnabled={serversEnabled} onOrderChange={changeNavigationOrder} onHiddenPagesChange={dashboardPreferences.setHiddenPages} onServersEnabledChange={dashboardPreferences.setServersEnabled} />
        <div class="dashboard-workspace">
          <Topbar section={active} hostname={hostname} refreshedAt={refreshedAt} user={user} notices={notices} onRefresh={data.refresh} onLogout={onLogout} theme={theme} onThemeChange={setTheme} />
          <MobileNav active={active} csrfToken={csrfToken} order={navigationOrder} hiddenPages={hiddenPages} serversEnabled={serversEnabled} />
          <main id="main-content" tabIndex={-1}>
            {active === 'overview' && <OverviewRoute data={data} themeLabel={themeLabel} csrfToken={csrfToken} canManageDocker={user.capabilities.includes('system.settings.write')} onSessionExpired={onSessionExpired} />}
            {active === 'home' && <HomeRoute overview={data.overview.data} inventory={data.inventory.data} servers={data.servers.data ?? []} displayName={user.displayName} templates={dashboardPreferences.homeTemplates} activeHomeId={dashboardPreferences.activeHomeId} syncStatus={dashboardPreferences.syncStatus} onHomeChange={dashboardPreferences.setHomeState} csrfToken={csrfToken} homeFocus={homeFocused} onHomeFocusToggle={toggleHomeFocus} canManageDocker={user.capabilities.includes('system.settings.write')} onSessionExpired={onSessionExpired} />}
            {active === 'storage' && <StoragePageRoute data={data} csrfToken={csrfToken} onSessionExpired={onSessionExpired} />}
            {active === 'network' && <NetworkPageRoute data={data} csrfToken={csrfToken} canManageFirewall={user.capabilities.includes('network.firewall.write')} onSessionExpired={onSessionExpired} />}
            {active === 'host' && <HostPageRoute data={data} csrfToken={csrfToken} canManageDocker={user.capabilities.includes('system.settings.write')} onSessionExpired={onSessionExpired} />}
            {active === 'security' && <SecurityRoute csrfToken={csrfToken} canManage={user.capabilities.includes('system.settings.write')} themeLabel={themeLabel} helixVersion={data.overview.data?.helixVersion ?? null} onSessionExpired={onSessionExpired} />}
            {active === 'terminal' && <TerminalRoute csrfToken={csrfToken} canOpen={user.capabilities.includes('terminal.open')} onSessionExpired={onSessionExpired} />}
            {active === 'servers' && (serversEnabled ? <ServersRoute data={data} csrfToken={csrfToken} canManageServers={user.capabilities.includes('games.manage')} canManageBackups={user.capabilities.includes('games.backups.manage')} canManageNetwork={user.capabilities.includes('network.firewall.write')} onSessionExpired={onSessionExpired} /> : <ServersModuleDisabled onEnable={() => dashboardPreferences.setServersEnabled(true)} />)}
            {active === 'hooks' && <HooksRoute csrfToken={csrfToken} canManage={user.capabilities.includes('system.settings.write')} onSessionExpired={onSessionExpired} />}
            {active === 'strands' && <StrandsRoute csrfToken={csrfToken} canManage={user.capabilities.includes('system.settings.write')} onSessionExpired={onSessionExpired} />}
            {active === 'globe' && <GlobeRoute csrfToken={csrfToken} onSessionExpired={onSessionExpired} />}
            {active === 'settings' && <SettingsRoute user={user} csrfToken={csrfToken} theme={theme} refreshIntervalMs={refreshIntervalMs} navigationOrder={navigationOrder} hiddenPages={hiddenPages} colors={dashboardPreferences.colors} serversEnabled={serversEnabled} preferenceSyncStatus={dashboardPreferences.syncStatus} hostIntegration={data.integration} servers={data.servers.data ?? []} onThemeChange={setTheme} onRefreshIntervalChange={changeRefreshInterval} onNavigationOrderChange={changeNavigationOrder} onHiddenPagesChange={dashboardPreferences.setHiddenPages} onColorsChange={dashboardPreferences.setColors} onServersEnabledChange={dashboardPreferences.setServersEnabled} onAccountUpdated={onAccountUpdated} onHostIntegrationRefresh={data.refresh} />}
          </main>
        </div>
      </div>
      <OperationContinuity csrfToken={csrfToken} onRefresh={data.refresh} onSessionExpired={onSessionExpired} />
    </>
  );
}
