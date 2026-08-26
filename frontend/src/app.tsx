import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { ApiError, getHealth, getSystemOverview } from './api';
import {
  calculatePercent,
  formatBytes,
  formatDuration,
  formatPercent,
  formatTimestamp,
} from './format';
import { focusOnMount } from './focus';
import { t, type TranslationId } from './i18n';
import {
  dashboardSectionForHash,
  type DashboardSectionId,
} from './navigation';
import { PROJECT_SOURCE_URL } from './project';
import {
  applyThemePreference,
  readThemePreference,
  saveThemePreference,
  themeOptions,
  type ThemePreference,
} from './theme';
import type {
  AuthenticatedUser,
  DiscoveryAvailability,
  HealthSnapshot,
  NetworkSnapshot,
  StorageMountSnapshot,
  StorageSnapshot,
  SystemOverview,
} from './types';

type LoadPhase = 'loading' | 'refreshing' | 'ready' | 'stale' | 'error';
type StatusTone = 'good' | 'warning' | 'danger' | 'neutral';

interface ResourceState<T> {
  data: T | null;
  phase: LoadPhase;
  error: string | null;
}

interface DashboardData {
  health: ResourceState<HealthSnapshot>;
  overview: ResourceState<SystemOverview>;
  refresh: () => Promise<void>;
  isRefreshing: boolean;
}

const REFRESH_INTERVAL_MS = 30_000;

const dashboardSectionLabelIds = {
  overview: 'dashboard.nav.overview',
  health: 'dashboard.nav.health',
  host: 'dashboard.nav.host',
  storage: 'dashboard.nav.storage',
  network: 'dashboard.nav.network',
} as const satisfies Record<DashboardSectionId, TranslationId>;

function createLoadingResource<T>(): ResourceState<T> {
  return { data: null, phase: 'loading', error: null };
}

function describeError(error: unknown): string {
  if (error instanceof ApiError) {
    return error.message;
  }

  return t('dashboard.error.fallback');
}

function useDashboardData(
  csrfToken: string,
  onSessionExpired: () => void,
): DashboardData {
  const [health, setHealth] = useState<ResourceState<HealthSnapshot>>(
    createLoadingResource,
  );
  const [overview, setOverview] = useState<ResourceState<SystemOverview>>(
    createLoadingResource,
  );
  const inFlight = useRef(false);
  const controller = useRef<AbortController | null>(null);
  const mounted = useRef(true);

  const refresh = useCallback(async (): Promise<void> => {
    if (inFlight.current) {
      return;
    }

    inFlight.current = true;
    controller.current?.abort();
    const nextController = new AbortController();
    controller.current = nextController;

    setHealth((current) => ({
      ...current,
      phase: current.data === null ? 'loading' : 'refreshing',
      error: null,
    }));
    setOverview((current) => ({
      ...current,
      phase: current.data === null ? 'loading' : 'refreshing',
      error: null,
    }));

    const [healthResult, overviewResult] = await Promise.allSettled([
      getHealth(csrfToken, nextController.signal),
      getSystemOverview(csrfToken, nextController.signal),
    ]);

    if (!mounted.current || nextController.signal.aborted) {
      inFlight.current = false;
      return;
    }

    const sessionExpired = [healthResult, overviewResult].some(
      (result) =>
        result.status === 'rejected' &&
        result.reason instanceof ApiError &&
        (result.reason.status === 401 || result.reason.code === 'csrf_rejected'),
    );
    if (sessionExpired) {
      inFlight.current = false;
      onSessionExpired();
      return;
    }

    if (healthResult.status === 'fulfilled') {
      setHealth({ data: healthResult.value, phase: 'ready', error: null });
    } else {
      const message = describeError(healthResult.reason);
      setHealth((current) => ({
        data: current.data,
        phase: current.data === null ? 'error' : 'stale',
        error: message,
      }));
    }

    if (overviewResult.status === 'fulfilled') {
      setOverview({ data: overviewResult.value, phase: 'ready', error: null });
    } else {
      const message = describeError(overviewResult.reason);
      setOverview((current) => ({
        data: current.data,
        phase: current.data === null ? 'error' : 'stale',
        error: message,
      }));
    }

    inFlight.current = false;
  }, [csrfToken, onSessionExpired]);

  useEffect(() => {
    mounted.current = true;
    void refresh();

    const interval = window.setInterval(() => {
      if (!document.hidden) {
        void refresh();
      }
    }, REFRESH_INTERVAL_MS);

    const handleVisibilityChange = (): void => {
      if (!document.hidden) {
        void refresh();
      }
    };
    document.addEventListener('visibilitychange', handleVisibilityChange);

    return () => {
      mounted.current = false;
      controller.current?.abort();
      window.clearInterval(interval);
      document.removeEventListener('visibilitychange', handleVisibilityChange);
    };
  }, [refresh]);

  return {
    health,
    overview,
    refresh,
    isRefreshing:
      health.phase === 'loading' ||
      health.phase === 'refreshing' ||
      overview.phase === 'loading' ||
      overview.phase === 'refreshing',
  };
}

function useTheme(): [ThemePreference, (theme: ThemePreference) => void] {
  const [theme, setTheme] = useState<ThemePreference>(readThemePreference);

  useEffect(() => {
    saveThemePreference(theme);
    const colorPreference = window.matchMedia('(prefers-color-scheme: light)');
    const updateResolvedTheme = (): void => {
      applyThemePreference(theme, colorPreference.matches);
    };

    updateResolvedTheme();
    if (theme === 'system') {
      colorPreference.addEventListener('change', updateResolvedTheme);
    }

    return () => {
      colorPreference.removeEventListener('change', updateResolvedTheme);
    };
  }, [theme]);

  return [theme, setTheme];
}

function useActiveSection(): DashboardSectionId {
  const [activeSection, setActiveSection] = useState<DashboardSectionId>(() =>
    typeof window === 'undefined' ? 'overview' : dashboardSectionForHash(window.location.hash),
  );

  useEffect(() => {
    const update = (): void => {
      setActiveSection(dashboardSectionForHash(window.location.hash));
    };

    window.addEventListener('hashchange', update);
    return () => {
      window.removeEventListener('hashchange', update);
    };
  }, []);

  return activeSection;
}

function normalizeStatus(status: string): string {
  return status.trim().toLowerCase();
}

function displayStatus(status: string): string {
  return status
    .trim()
    .replaceAll(/[_-]+/g, ' ')
    .replace(/^./, (character) => character.toUpperCase());
}

function healthTone(health: HealthSnapshot): StatusTone {
  const statuses = [
    health.status,
    health.stateDatabase,
    health.metricsDatabase,
  ].map(normalizeStatus);

  return statuses.every((status) => status === 'ok') ? 'good' : 'warning';
}

function dashboardStatus(
  health: ResourceState<HealthSnapshot>,
): { label: string; detail: string; tone: StatusTone } {
  if (health.data !== null) {
    const tone = health.phase === 'stale' ? 'warning' : healthTone(health.data);
    const label =
      health.phase === 'stale'
        ? t('dashboard.status.lastKnown')
        : displayStatus(health.data.status);
    const detail =
      health.phase === 'stale'
        ? t('dashboard.status.refreshFailed')
        : tone === 'good'
          ? t('dashboard.status.operational')
          : t('dashboard.status.attention');
    return { label, detail, tone };
  }

  if (health.phase === 'error') {
    return {
      label: t('dashboard.status.unavailable'),
      detail: t('dashboard.status.healthUnavailable'),
      tone: 'danger',
    };
  }

  return {
    label: t('dashboard.status.connecting'),
    detail: t('dashboard.status.waiting'),
    tone: 'neutral',
  };
}

function StatusPill({
  label,
  tone,
}: {
  label: string;
  tone: StatusTone;
}) {
  return (
    <span class={`status-pill status-pill--${tone}`}>
      <span class="status-pill__dot" aria-hidden="true" />
      {label}
    </span>
  );
}

function Brand() {
  return (
    <a class="brand" href="#overview" aria-label={t('dashboard.brandOverview')}>
      <span class="brand__mark" aria-hidden="true">
        <span />
        <span />
      </span>
      <span class="brand__wordmark">HELIX</span>
    </a>
  );
}

function Sidebar({
  status,
}: {
  status: ReturnType<typeof dashboardStatus>;
}) {
  const activeSection = useActiveSection();

  return (
    <aside class="sidebar">
      <Brand />
      <nav class="primary-nav" aria-label={t('dashboard.navigation')}>
        <a
          class={`primary-nav__link${activeSection === 'overview' ? ' primary-nav__link--active' : ''}`}
          href="#overview"
          aria-current={activeSection === 'overview' ? 'location' : undefined}
        >
          <span class="primary-nav__index" aria-hidden="true">01</span>
          {t('dashboard.nav.overview')}
        </a>
        <a
          class={`primary-nav__link${activeSection === 'health' ? ' primary-nav__link--active' : ''}`}
          href="#health"
          aria-current={activeSection === 'health' ? 'location' : undefined}
        >
          <span class="primary-nav__index" aria-hidden="true">02</span>
          {t('dashboard.nav.health')}
        </a>
        <a
          class={`primary-nav__link${activeSection === 'host' ? ' primary-nav__link--active' : ''}`}
          href="#host"
          aria-current={activeSection === 'host' ? 'location' : undefined}
        >
          <span class="primary-nav__index" aria-hidden="true">03</span>
          {t('dashboard.nav.host')}
        </a>
        <a
          class={`primary-nav__link${activeSection === 'storage' ? ' primary-nav__link--active' : ''}`}
          href="#storage"
          aria-current={activeSection === 'storage' ? 'location' : undefined}
        >
          <span class="primary-nav__index" aria-hidden="true">04</span>
          {t('dashboard.nav.storage')}
        </a>
        <a
          class={`primary-nav__link${activeSection === 'network' ? ' primary-nav__link--active' : ''}`}
          href="#network"
          aria-current={activeSection === 'network' ? 'location' : undefined}
        >
          <span class="primary-nav__index" aria-hidden="true">05</span>
          {t('dashboard.nav.network')}
        </a>
      </nav>
      <div class="sidebar__status">
        <span class="eyebrow">{t('dashboard.controlPlane')}</span>
        <StatusPill label={status.label} tone={status.tone} />
        <span class="sidebar__status-detail">{status.detail}</span>
      </div>
    </aside>
  );
}

function ThemeSelector({
  theme,
  onChange,
}: {
  theme: ThemePreference;
  onChange: (theme: ThemePreference) => void;
}) {
  return (
    <label class="theme-selector">
      <span class="sr-only">{t('common.colorTheme')}</span>
      <span class="theme-selector__swatch" aria-hidden="true" />
      <select
        value={theme}
        onChange={(event) => onChange(event.currentTarget.value as ThemePreference)}
        aria-label={t('common.colorTheme')}
      >
        {themeOptions.map((option) => (
          <option key={option.value} value={option.value}>
            {t(option.labelId)}
          </option>
        ))}
      </select>
    </label>
  );
}

function DashboardThemeSelector() {
  const [theme, setTheme] = useTheme();
  return <ThemeSelector theme={theme} onChange={setTheme} />;
}

function CurrentSectionLabel() {
  const activeSection = useActiveSection();
  return <span>{t(dashboardSectionLabelIds[activeSection])}</span>;
}

function MobileNavigation() {
  const activeSection = useActiveSection();

  return (
    <nav class="mobile-nav" aria-label={t('dashboard.navigation')}>
      <a
        class={activeSection === 'overview' ? 'mobile-nav__link--active' : undefined}
        href="#overview"
        aria-current={activeSection === 'overview' ? 'location' : undefined}
      >
        {t('dashboard.nav.overview')}
      </a>
      <a
        class={activeSection === 'health' ? 'mobile-nav__link--active' : undefined}
        href="#health"
        aria-current={activeSection === 'health' ? 'location' : undefined}
      >
        {t('dashboard.nav.health')}
      </a>
      <a
        class={activeSection === 'host' ? 'mobile-nav__link--active' : undefined}
        href="#host"
        aria-current={activeSection === 'host' ? 'location' : undefined}
      >
        {t('dashboard.nav.host')}
      </a>
      <a
        class={activeSection === 'storage' ? 'mobile-nav__link--active' : undefined}
        href="#storage"
        aria-current={activeSection === 'storage' ? 'location' : undefined}
      >
        {t('dashboard.nav.storage')}
      </a>
      <a
        class={activeSection === 'network' ? 'mobile-nav__link--active' : undefined}
        href="#network"
        aria-current={activeSection === 'network' ? 'location' : undefined}
      >
        {t('dashboard.nav.network')}
      </a>
    </nav>
  );
}

function ResourceNotice({
  resource,
  label,
}: {
  resource: ResourceState<unknown>;
  label: string;
}) {
  if (resource.error === null) {
    return null;
  }

  return (
    <div
      class={`resource-notice resource-notice--${resource.data === null ? 'danger' : 'warning'}`}
      role={resource.data === null ? 'alert' : 'status'}
    >
      <strong>
        {resource.data === null
          ? t('dashboard.resourceUnavailable', label)
          : t('dashboard.resourceStale', label)}
      </strong>
      <span>{resource.error}</span>
    </div>
  );
}

function LoadingMetric() {
  return (
    <div class="metric-skeleton" aria-label={t('dashboard.loadingMetric')}>
      <span />
      <span />
      <span />
    </div>
  );
}

function MetricCard({
  label,
  value,
  detail,
  percentage,
  loading = false,
  unavailable = false,
}: {
  label: string;
  value: string;
  detail: string;
  percentage?: number | null;
  loading?: boolean;
  unavailable?: boolean;
}) {
  const safePercentage =
    percentage === undefined || percentage === null
      ? null
      : Math.min(100, Math.max(0, percentage));

  return (
    <div class={`metric-card${unavailable ? ' metric-card--unavailable' : ''}`}>
      <span class="metric-card__label">{label}</span>
      {loading ? (
        <LoadingMetric />
      ) : (
        <>
          <strong class="metric-card__value">{value}</strong>
          <span class="metric-card__detail">{detail}</span>
          {safePercentage !== null && (
            <progress
              class="meter"
              max={100}
              value={safePercentage}
              aria-label={t(
                'dashboard.metricAria',
                label,
                formatPercent(percentage ?? safePercentage),
              )}
            />
          )}
        </>
      )}
    </div>
  );
}

function MetricGrid({ overview }: { overview: ResourceState<SystemOverview> }) {
  const data = overview.data;
  const isInitialLoad = data === null && overview.phase === 'loading';

  if (data === null) {
    const unavailable = overview.phase === 'error';
    const value = unavailable ? t('dashboard.status.unavailable') : '';
    const detail = unavailable ? t('dashboard.metric.unavailableDetail') : '';
    const labels = [
      t('dashboard.metric.cpu'),
      t('dashboard.metric.memory'),
      t('dashboard.metric.uptime'),
      t('dashboard.metric.swap'),
    ];
    return (
      <div class="metric-grid">
        {labels.map((label) => (
          <MetricCard
            key={label}
            label={label}
            value={value}
            detail={detail}
            loading={isInitialLoad}
            unavailable={unavailable}
          />
        ))}
      </div>
    );
  }

  const memoryPercent = calculatePercent(data.memory.usedBytes, data.memory.totalBytes);
  const swapPercent = calculatePercent(data.swap.usedBytes, data.swap.totalBytes);
  const cpuValue = data.cpu.usagePercent;

  return (
    <div class="metric-grid">
      <MetricCard
        label={t('dashboard.metric.cpu')}
        value={cpuValue === null ? t('dashboard.metric.cpuSampling') : formatPercent(cpuValue)}
        detail={
          cpuValue === null
            ? t('dashboard.metric.cpuWaiting')
            : t('dashboard.metric.logicalCores', data.cpu.logicalCores)
        }
        percentage={cpuValue}
      />
      <MetricCard
        label={t('dashboard.metric.memory')}
        value={
          data.memory.totalBytes === 0
            ? t('dashboard.metric.notReported')
            : formatPercent(memoryPercent ?? 0)
        }
        detail={
          data.memory.totalBytes === 0
            ? t('dashboard.metric.zeroMemory')
            : t(
                'dashboard.metric.usedOfTotal',
                formatBytes(data.memory.usedBytes),
                formatBytes(data.memory.totalBytes),
              )
        }
        percentage={memoryPercent}
      />
      <MetricCard
        label={t('dashboard.metric.uptime')}
        value={formatDuration(data.uptimeSeconds)}
        detail={t('dashboard.metric.reportedByHost')}
      />
      <MetricCard
        label={t('dashboard.metric.swap')}
        value={
          data.swap.totalBytes === 0
            ? t('dashboard.metric.swapNotConfigured')
            : formatPercent(swapPercent ?? 0)
        }
        detail={
          data.swap.totalBytes === 0
            ? t('dashboard.metric.noSwap')
            : t(
                'dashboard.metric.usedOfTotal',
                formatBytes(data.swap.usedBytes),
                formatBytes(data.swap.totalBytes),
              )
        }
        percentage={swapPercent}
      />
    </div>
  );
}

function DetailRow({
  label,
  value,
  tone = 'neutral',
}: {
  label: string;
  value: string;
  tone?: StatusTone;
}) {
  return (
    <div class="detail-row">
      <dt>{label}</dt>
      <dd>
        {tone === 'neutral' ? value : <StatusPill label={value} tone={tone} />}
      </dd>
    </div>
  );
}

function PanelSkeleton({ rows = 4 }: { rows?: number }) {
  return (
    <div class="panel-skeleton" aria-label={t('dashboard.loadingDetails')}>
      {Array.from({ length: rows }, (_, index) => (
        <span key={index} />
      ))}
    </div>
  );
}

function HealthPanel({ health }: { health: ResourceState<HealthSnapshot> }) {
  const data = health.data;
  return (
    <section class="panel" id="health" aria-labelledby="health-title">
      <div class="panel__heading">
        <div>
          <span class="eyebrow">{t('dashboard.controlPlane')}</span>
          <h2 id="health-title">{t('dashboard.health.title')}</h2>
        </div>
        {data !== null && <StatusPill label={displayStatus(data.status)} tone={healthTone(data)} />}
      </div>
      <ResourceNotice resource={health} label={t('dashboard.health.title')} />
      {data === null ? (
        health.phase === 'loading' ? (
          <PanelSkeleton />
        ) : (
          <div class="empty-state">
            <strong>{t('dashboard.health.detailsUnavailable')}</strong>
            <span>{t('dashboard.health.noValues')}</span>
          </div>
        )
      ) : (
        <dl class="detail-list">
          <DetailRow
            label={t('dashboard.health.apiStatus')}
            value={displayStatus(data.status)}
            tone={normalizeStatus(data.status) === 'ok' ? 'good' : 'warning'}
          />
          <DetailRow
            label={t('dashboard.health.stateDatabase')}
            value={displayStatus(data.stateDatabase)}
            tone={normalizeStatus(data.stateDatabase) === 'ok' ? 'good' : 'warning'}
          />
          <DetailRow
            label={t('dashboard.health.metricsDatabase')}
            value={displayStatus(data.metricsDatabase)}
            tone={normalizeStatus(data.metricsDatabase) === 'ok' ? 'good' : 'warning'}
          />
          <DetailRow label={t('dashboard.health.version')} value={data.version} />
          <DetailRow
            label={t('dashboard.health.checked')}
            value={formatTimestamp(data.timestampUnixMs)}
          />
        </dl>
      )}
    </section>
  );
}

export function HostPanel({ overview }: { overview: ResourceState<SystemOverview> }) {
  const data = overview.data;
  return (
    <section class="panel" id="host" aria-labelledby="host-title">
      <div class="panel__heading">
        <div>
          <span class="eyebrow">{t('dashboard.host.eyebrow')}</span>
          <h2 id="host-title">{t('dashboard.host.title')}</h2>
        </div>
        {data !== null && <span class="architecture-tag">{data.architecture}</span>}
      </div>
      <ResourceNotice resource={overview} label={t('dashboard.host.title')} />
      {data === null ? (
        overview.phase === 'loading' ? (
          <PanelSkeleton rows={6} />
        ) : (
          <div class="empty-state">
            <strong>{t('dashboard.host.detailsUnavailable')}</strong>
            <span>{t('dashboard.host.noValues')}</span>
          </div>
        )
      ) : (
        <dl class="detail-list">
          <DetailRow
            label={t('dashboard.host.hostname')}
            value={data.hostname ?? t('dashboard.metric.notReported')}
          />
          <DetailRow
            label={t('dashboard.host.operatingSystem')}
            value={data.operatingSystem ?? t('dashboard.metric.notReported')}
          />
          <DetailRow label={t('dashboard.host.architecture')} value={data.architecture} />
          <DetailRow
            label={t('dashboard.host.kernel')}
            value={data.kernelVersion ?? t('dashboard.metric.notReported')}
          />
          <DetailRow
            label={t('dashboard.host.logicalCores')}
            value={String(data.cpu.logicalCores)}
          />
          <DetailRow
            label={t('dashboard.host.availableMemory')}
            value={formatBytes(data.memory.availableBytes)}
          />
          <DetailRow
            label={t('dashboard.host.sampled')}
            value={formatTimestamp(data.collectedAtUnixMs)}
          />
        </dl>
      )}
    </section>
  );
}

function discoveryStatus(
  availability: DiscoveryAvailability,
): { label: string; tone: StatusTone } {
  switch (availability) {
    case 'available':
      return { label: t('dashboard.discovery.available'), tone: 'good' };
    case 'degraded':
      return { label: t('dashboard.discovery.degraded'), tone: 'warning' };
    case 'unavailable':
      return { label: t('dashboard.discovery.unavailable'), tone: 'neutral' };
  }
}

function DiscoveryNotice({ messages }: { messages: string[] }) {
  if (messages.length === 0) {
    return null;
  }

  return (
    <div class="discovery-notice" role="status">
      <strong>{t('dashboard.discovery.partial')}</strong>
      <ul>
        {messages.map((message) => <li key={message}>{message}</li>)}
      </ul>
    </div>
  );
}

function StorageMountCard({ mount, index }: { mount: StorageMountSnapshot; index: number }) {
  const title =
    mount.mountPoint ?? mount.name ?? `${t('dashboard.storage.unknownMount')} ${index + 1}`;
  const usedPercent = calculatePercent(mount.usedBytes, mount.totalBytes);

  return (
    <li class="discovery-card">
      <div class="discovery-card__content">
        <header class="discovery-card__header">
          <div>
            <strong title={title}>{title}</strong>
            <span>{mount.fileSystem ?? t('dashboard.metric.notReported')}</span>
          </div>
          <div class="discovery-card__flags">
            {mount.readOnly && <span>{t('dashboard.storage.readOnly')}</span>}
            {mount.removable && <span>{t('dashboard.storage.removable')}</span>}
          </div>
        </header>

        <div class="capacity-summary">
          <div>
            <span>{t('dashboard.storage.capacity')}</span>
            <strong>
              {mount.totalBytes === 0n
                ? t('dashboard.metric.notReported')
                : t(
                    'dashboard.metric.usedOfTotal',
                    formatBytes(mount.usedBytes),
                    formatBytes(mount.totalBytes),
                  )}
            </strong>
          </div>
          <div>
            <span>{t('dashboard.storage.available')}</span>
            <strong>{formatBytes(mount.availableBytes)}</strong>
          </div>
        </div>
        {usedPercent !== null && (
          <progress
            class="meter discovery-card__meter"
            max={100}
            value={Math.min(100, Math.max(0, usedPercent))}
            aria-label={t(
              'dashboard.metricAria',
              title,
              formatPercent(usedPercent),
            )}
          />
        )}

        <dl class="discovery-card__details">
          <div>
            <dt>{t('dashboard.storage.device')}</dt>
            <dd>{mount.name ?? t('dashboard.metric.notReported')}</dd>
          </div>
          <div>
            <dt>{t('dashboard.storage.mountPoint')}</dt>
            <dd>{mount.mountPoint ?? t('dashboard.metric.notReported')}</dd>
          </div>
          <div>
            <dt>{t('dashboard.storage.fileSystem')}</dt>
            <dd>{mount.fileSystem ?? t('dashboard.metric.notReported')}</dd>
          </div>
          <div>
            <dt>{t('dashboard.storage.readOnly')}</dt>
            <dd>
              {mount.readOnly ? t('dashboard.common.yes') : t('dashboard.common.no')}
            </dd>
          </div>
          <div>
            <dt>{t('dashboard.storage.removable')}</dt>
            <dd>
              {mount.removable ? t('dashboard.common.yes') : t('dashboard.common.no')}
            </dd>
          </div>
        </dl>
      </div>
    </li>
  );
}

export function StoragePanel({ overview }: { overview: ResourceState<SystemOverview> }) {
  const storage: StorageSnapshot | null = overview.data?.storage ?? null;
  const status = storage === null ? null : discoveryStatus(storage.availability);
  const notices = storage === null
    ? []
    : [
        storage.omittedMounts > 0
          ? t('dashboard.storage.omittedMounts', storage.omittedMounts)
          : null,
        storage.omittedTextFields > 0
          ? t('dashboard.storage.omittedText', storage.omittedTextFields)
          : null,
      ].filter((message): message is string => message !== null);

  return (
    <section class="panel discovery-panel" id="storage" aria-labelledby="storage-title">
      <div class="panel__heading">
        <div>
          <span class="eyebrow">{t('dashboard.storage.eyebrow')}</span>
          <h2 id="storage-title">{t('dashboard.storage.title')}</h2>
        </div>
        {status !== null && <StatusPill label={status.label} tone={status.tone} />}
      </div>
      {storage !== null && storage.availability !== 'unavailable' && (
        <p class="discovery-panel__summary">
          {t('dashboard.storage.mountCount', storage.mounts.length)}
        </p>
      )}
      <DiscoveryNotice messages={notices} />
      {storage === null ? (
        overview.phase === 'loading' ? (
          <PanelSkeleton rows={4} />
        ) : (
          <div class="empty-state">
            <strong>{t('dashboard.storage.detailsUnavailable')}</strong>
            <span>{t('dashboard.storage.noValues')}</span>
          </div>
        )
      ) : storage.availability === 'unavailable' ? (
        <div class="empty-state">
          <strong>{t('dashboard.storage.unavailable')}</strong>
          <span>{t('dashboard.storage.unavailableDetail')}</span>
        </div>
      ) : storage.mounts.length === 0 ? (
        <div class="empty-state">
          <strong>{t('dashboard.storage.noMounts')}</strong>
          <span>{t('dashboard.storage.noMountsDetail')}</span>
        </div>
      ) : (
        <ul class="discovery-list">
          {storage.mounts.map((mount, index) => (
            <StorageMountCard
              key={`${mount.mountPoint ?? ''}:${mount.name ?? ''}:${index}`}
              mount={mount}
              index={index}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

export function NetworkPanel({ overview }: { overview: ResourceState<SystemOverview> }) {
  const network: NetworkSnapshot | null = overview.data?.network ?? null;
  const status = network === null ? null : discoveryStatus(network.availability);
  const notices = network === null
    ? []
    : [
        network.omittedInterfaces > 0
          ? t('dashboard.network.omittedInterfaces', network.omittedInterfaces)
          : null,
        network.omittedAddresses > 0
          ? t('dashboard.network.omittedAddresses', network.omittedAddresses)
          : null,
      ].filter((message): message is string => message !== null);

  return (
    <section class="panel discovery-panel" id="network" aria-labelledby="network-title">
      <div class="panel__heading">
        <div>
          <span class="eyebrow">{t('dashboard.network.eyebrow')}</span>
          <h2 id="network-title">{t('dashboard.network.title')}</h2>
        </div>
        {status !== null && <StatusPill label={status.label} tone={status.tone} />}
      </div>
      {network !== null && network.availability !== 'unavailable' && (
        <div class="discovery-panel__summary">
          <span>{t('dashboard.network.interfaceCount', network.interfaces.length)}</span>
          <span>{t('dashboard.network.cumulativeHelp')}</span>
        </div>
      )}
      <DiscoveryNotice messages={notices} />
      {network === null ? (
        overview.phase === 'loading' ? (
          <PanelSkeleton rows={4} />
        ) : (
          <div class="empty-state">
            <strong>{t('dashboard.network.detailsUnavailable')}</strong>
            <span>{t('dashboard.network.noValues')}</span>
          </div>
        )
      ) : network.availability === 'unavailable' ? (
        <div class="empty-state">
          <strong>{t('dashboard.network.unavailable')}</strong>
          <span>{t('dashboard.network.unavailableDetail')}</span>
        </div>
      ) : network.interfaces.length === 0 ? (
        <div class="empty-state">
          <strong>{t('dashboard.network.noInterfaces')}</strong>
          <span>{t('dashboard.network.noInterfacesDetail')}</span>
        </div>
      ) : (
        <ul class="discovery-list">
          {network.interfaces.map((networkInterface) => (
            <li class="discovery-card" key={networkInterface.name}>
              <div class="discovery-card__content">
                <header class="discovery-card__header">
                  <div>
                    <strong>{networkInterface.name}</strong>
                    <span>
                      {t('dashboard.network.mtu')} · {formatBytes(networkInterface.mtuBytes)}
                    </span>
                  </div>
                </header>

                <div class="network-counters">
                  <div>
                    <span>{t('dashboard.network.received')}</span>
                    <strong>{formatBytes(networkInterface.totalReceivedBytes)}</strong>
                  </div>
                  <div>
                    <span>{t('dashboard.network.transmitted')}</span>
                    <strong>{formatBytes(networkInterface.totalTransmittedBytes)}</strong>
                  </div>
                </div>

                <div class="address-block">
                  <span>{t('dashboard.network.addresses')}</span>
                  {networkInterface.addresses.length === 0 ? (
                    <strong>{t('dashboard.network.noAddresses')}</strong>
                  ) : (
                    <ul class="address-list">
                      {networkInterface.addresses.map((address, index) => (
                        <li key={`${address.address}/${address.prefixLength}:${index}`}>
                          {address.address}/{address.prefixLength}
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export interface DashboardProps {
  user: AuthenticatedUser;
  csrfToken: string;
  onSessionExpired: () => void;
  onLogout: () => Promise<void>;
}

export function Dashboard({
  user,
  csrfToken,
  onSessionExpired,
  onLogout,
}: DashboardProps) {
  const { health, overview, refresh, isRefreshing } =
    useDashboardData(csrfToken, onSessionExpired);
  const [isLoggingOut, setIsLoggingOut] = useState(false);
  const [logoutError, setLogoutError] = useState<string | null>(null);
  const status = dashboardStatus(health);
  const hostLabel = overview.data?.hostname ?? t('dashboard.localServer');

  const handleLogout = async (): Promise<void> => {
    if (isLoggingOut) {
      return;
    }
    setIsLoggingOut(true);
    setLogoutError(null);
    try {
      await onLogout();
    } catch (error) {
      setLogoutError(describeError(error));
      setIsLoggingOut(false);
    }
  };

  return (
    <>
      <a class="skip-link" href="#main-content">{t('dashboard.skipToMain')}</a>
      <div class="app-shell">
        <Sidebar status={status} />
        <div class="workspace">
          <header class="topbar">
            <div class="topbar__mobile-brand">
              <Brand />
            </div>
            <div class="topbar__context">
              <span class="topbar__host">{hostLabel}</span>
              <span class="topbar__separator" aria-hidden="true">/</span>
              <CurrentSectionLabel />
            </div>
            <div class="topbar__actions">
              <span class="user-chip" title={t('dashboard.signedInAs', user.loginName)}>
                {user.displayName}
              </span>
              <DashboardThemeSelector />
              <button
                class="refresh-button"
                type="button"
                disabled={isRefreshing}
                onClick={() => void refresh()}
                aria-busy={isRefreshing}
                aria-disabled={isRefreshing}
              >
                <span class={isRefreshing ? 'refresh-button__icon is-spinning' : 'refresh-button__icon'} aria-hidden="true">↻</span>
                <span>
                  {isRefreshing ? t('dashboard.refreshing') : t('dashboard.refresh')}
                </span>
              </button>
              <button
                class="logout-button"
                type="button"
                disabled={isLoggingOut}
                onClick={() => void handleLogout()}
                aria-busy={isLoggingOut}
                aria-disabled={isLoggingOut}
                aria-label={t('dashboard.signOut')}
              >
                <span class="logout-button__icon" aria-hidden="true">↪</span>
                <span class="logout-button__label">
                  {isLoggingOut ? t('dashboard.signingOut') : t('dashboard.signOut')}
                </span>
              </button>
            </div>
          </header>
          {logoutError !== null && (
            <div class="topbar-error" role="alert">
              {logoutError}
            </div>
          )}
          <MobileNavigation />

          <main id="main-content" tabIndex={-1}>
            <section class="hero" id="overview" aria-labelledby="overview-title">
              <div class="hero__copy">
                <span class="eyebrow">{t('dashboard.hero.eyebrow')}</span>
                <h1 id="overview-title" ref={focusOnMount} tabIndex={-1}>
                  {t('dashboard.hero.titleBefore')}
                  <span> {t('dashboard.hero.titleAfter')}</span>
                </h1>
                <p>{t('dashboard.hero.detail')}</p>
              </div>
              <div
                class="hero__status-card"
                role="status"
                aria-live="polite"
                aria-atomic="true"
              >
                <div class="hero__status-topline">
                  <span class="eyebrow">{t('dashboard.hero.currentStatus')}</span>
                  <span class="pulse-mark" aria-hidden="true" />
                </div>
                <strong>{status.label}</strong>
                <span>{status.detail}</span>
                <div class="hero__status-footer">
                  <span>{t('dashboard.hero.refreshCadence')}</span>
                  <span>{t('dashboard.hero.refreshVisible')}</span>
                </div>
              </div>
            </section>

            <ResourceNotice resource={overview} label={t('dashboard.systemOverview')} />
            <MetricGrid overview={overview} />

            <div class="panel-grid">
              <HealthPanel health={health} />
              <HostPanel overview={overview} />
            </div>

            <div class="panel-grid panel-grid--discovery">
              <StoragePanel overview={overview} />
              <NetworkPanel overview={overview} />
            </div>

            <p class="refresh-announcement" aria-live="polite" aria-atomic="true">
              {isRefreshing ? t('dashboard.refreshAnnouncement') : ''}
            </p>
          </main>

          <footer>
            <span>{t('dashboard.footer.phase')}</span>
            <span>
              {t('dashboard.footer.localFirst')} ·{' '}
              <a
                class="source-link"
                href={PROJECT_SOURCE_URL}
                target="_blank"
                rel="noopener noreferrer"
              >
                {t('common.sourceAndLicense')}
              </a>
            </span>
          </footer>
        </div>
      </div>
    </>
  );
}
