import { useCallback, useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { ApiError } from './api';
import { focusOnMount } from './focus';
import { formatBytes, formatDuration, formatPercent, formatTimestamp } from './format';
import { getGameHostingReadiness } from './game-api';
import type {
  GameBackupStatus,
  GameHostingBlocker,
  GameHostingReadiness,
  GameInstanceFeature,
  GameInstanceStatus,
  GameInstanceSummary,
  GameUpdateStatus,
} from './game-types';
import { t, type TranslationId } from './game-i18n';
import './games.css';

type ReadinessPhase = 'loading' | 'ready' | 'error';
type GameViewMode = 'cards' | 'compact';
type GameFilter = 'all' | 'online' | 'offline' | 'attention';
type GameTone = 'good' | 'warning' | 'danger' | 'neutral';

interface ReadinessState {
  data: GameHostingReadiness | null;
  phase: ReadinessPhase;
  error: string | null;
}

export interface GamesWorkspaceProps {
  csrfToken: string;
  onSessionExpired: () => void;
}

export interface GameWorkspaceViewProps {
  readiness: GameHostingReadiness | null;
  readinessPhase: ReadinessPhase;
  readinessError?: string | null;
  instances: readonly GameInstanceSummary[] | null;
  totalInstances?: number;
  onRetry?: () => void;
}

const statusLabelIds = {
  online: 'games.status.online',
  starting: 'games.status.starting',
  stopping: 'games.status.stopping',
  offline: 'games.status.offline',
  installing: 'games.status.installing',
  updating: 'games.status.updating',
  backing_up: 'games.status.backing_up',
  restoring: 'games.status.restoring',
  degraded: 'games.status.degraded',
  failed: 'games.status.failed',
  unknown: 'games.status.unknown',
} as const satisfies Record<GameInstanceStatus, TranslationId>;

const updateLabelIds = {
  current: 'games.update.current',
  available: 'games.update.available',
  pinned: 'games.update.pinned',
  checking: 'games.update.checking',
  unknown: 'games.update.unknown',
} as const satisfies Record<GameUpdateStatus, TranslationId>;

const backupLabelIds = {
  healthy: 'games.backup.healthy',
  stale: 'games.backup.stale',
  failed: 'games.backup.failed',
  unconfigured: 'games.backup.unconfigured',
  unknown: 'games.backup.unknown',
} as const satisfies Record<GameBackupStatus, TranslationId>;

const featureLabelIds = {
  console: 'games.tab.console',
  players: 'games.tab.players',
  settings: 'games.tab.settings',
  mods_plugins: 'games.tab.mods_plugins',
  worlds_saves: 'games.tab.worlds_saves',
  files: 'games.tab.files',
  networking: 'games.tab.networking',
  backups: 'games.tab.backups',
  automation: 'games.tab.automation',
  logs: 'games.tab.logs',
  performance: 'games.tab.performance',
  advanced: 'games.tab.advanced',
} as const satisfies Record<GameInstanceFeature, TranslationId>;

const blockerCopyIds = {
  verified_restore: {
    title: 'games.blocker.verified_restore.title',
    detail: 'games.blocker.verified_restore.detail',
  },
  privileged_broker: {
    title: 'games.blocker.privileged_broker.title',
    detail: 'games.blocker.privileged_broker.detail',
  },
  native_execution: {
    title: 'games.blocker.native_execution.title',
    detail: 'games.blocker.native_execution.detail',
  },
} as const;

function describeError(error: unknown): string {
  return error instanceof ApiError ? error.message : t('games.error.fallback');
}

function statusTone(status: GameInstanceStatus): GameTone {
  if (status === 'online') return 'good';
  if (['failed'].includes(status)) return 'danger';
  if (
    ['starting', 'stopping', 'installing', 'updating', 'backing_up', 'restoring', 'degraded']
      .includes(status)
  ) {
    return 'warning';
  }
  return 'neutral';
}

function instanceNeedsAttention(instance: GameInstanceSummary): boolean {
  return (
    instance.status === 'failed' ||
    instance.status === 'degraded' ||
    instance.health === 'degraded' ||
    instance.health === 'unavailable' ||
    instance.backupStatus === 'failed' ||
    instance.backupStatus === 'stale' ||
    instance.updateStatus === 'available' ||
    instance.warnings.length > 0
  );
}

export function filterGameInstances(
  instances: readonly GameInstanceSummary[],
  query: string,
  filter: GameFilter,
): GameInstanceSummary[] {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  return instances.filter((instance) => {
    const matchesQuery =
      normalizedQuery.length === 0 ||
      [instance.name, instance.game, instance.software, instance.version, instance.address ?? '']
        .some((value) => value.toLocaleLowerCase().includes(normalizedQuery));
    if (!matchesQuery) return false;
    if (filter === 'online') return instance.status === 'online';
    if (filter === 'offline') return instance.status === 'offline';
    if (filter === 'attention') return instanceNeedsAttention(instance);
    return true;
  });
}

function useGameRoute(): string | null {
  const readRoute = (): string | null => {
    if (typeof window === 'undefined') return null;
    const match = /^#games\/([^/?#]+)$/.exec(window.location.hash);
    if (match?.[1] === undefined) return null;
    try {
      return decodeURIComponent(match[1]);
    } catch {
      return null;
    }
  };
  const [instanceId, setInstanceId] = useState<string | null>(readRoute);
  useEffect(() => {
    const update = (): void => setInstanceId(readRoute());
    window.addEventListener('hashchange', update);
    return () => window.removeEventListener('hashchange', update);
  }, []);
  return instanceId;
}

function GameStatus({ status }: { status: GameInstanceStatus }) {
  return (
    <span class={`game-status game-status--${statusTone(status)}`}>
      <span aria-hidden="true" />
      {t(statusLabelIds[status])}
    </span>
  );
}

function SummaryMetric({ label, value }: { label: string; value: string | number }) {
  return (
    <div class="game-summary__metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function GameSummary({ instances }: { instances: readonly GameInstanceSummary[] | null }) {
  if (instances === null) {
    const unavailable = t('games.summary.unavailable');
    return (
      <section class="game-summary" aria-label={t('games.hero.eyebrow')}>
        <SummaryMetric label={t('games.summary.registered')} value={unavailable} />
        <SummaryMetric label={t('games.summary.online')} value={unavailable} />
        <SummaryMetric label={t('games.summary.players')} value={unavailable} />
        <SummaryMetric label={t('games.summary.attention')} value={unavailable} />
      </section>
    );
  }

  const online = instances.filter((instance) => instance.status === 'online').length;
  const players = instances.reduce((sum, instance) => sum + instance.playersOnline, 0);
  const attention = instances.filter(instanceNeedsAttention).length;
  return (
    <section class="game-summary" aria-label={t('games.hero.eyebrow')}>
      <SummaryMetric label={t('games.summary.registered')} value={instances.length} />
      <SummaryMetric label={t('games.summary.online')} value={online} />
      <SummaryMetric label={t('games.summary.players')} value={players} />
      <SummaryMetric label={t('games.summary.attention')} value={attention} />
    </section>
  );
}

function blockerCopy(blocker: GameHostingBlocker): { title: string; detail: string } {
  const known = blockerCopyIds[blocker.code as keyof typeof blockerCopyIds];
  if (known === undefined) {
    return {
      title: t('games.blocker.unknown.title'),
      detail: t('games.blocker.unknown.detail'),
    };
  }
  return { title: t(known.title), detail: t(known.detail) };
}

function ReadinessPanel({
  readiness,
  phase,
  error,
  onRetry,
}: {
  readiness: GameHostingReadiness | null;
  phase: ReadinessPhase;
  error: string | null;
  onRetry?: () => void;
}) {
  const availability = readiness?.availability ?? 'unavailable';
  const title =
    phase === 'loading'
      ? t('games.readiness.loading')
      : phase === 'error'
        ? t('games.readiness.failed')
        : availability === 'ready'
          ? t('games.readiness.ready')
          : availability === 'degraded'
            ? t('games.readiness.degraded')
            : t('games.readiness.unavailable');
  const detail =
    phase === 'error'
      ? (error ?? t('games.error.fallback'))
      : availability === 'ready'
        ? t('games.readiness.readyDetail')
        : availability === 'degraded'
          ? t('games.readiness.degradedDetail')
          : t('games.readiness.unavailableDetail');

  return (
    <section
      class={`game-readiness game-readiness--${availability}`}
      aria-labelledby="game-readiness-title"
      aria-busy={phase === 'loading'}
    >
      <div class="game-readiness__heading">
        <div>
          <span class="eyebrow">{t('games.readiness.eyebrow')}</span>
          <h2 id="game-readiness-title">{title}</h2>
          <p>{detail}</p>
        </div>
        {readiness !== null && (
          <span class="game-readiness__checked">
            {t('games.readiness.checked', formatTimestamp(readiness.collectedAtUnixMs))}
          </span>
        )}
        {phase === 'error' && onRetry !== undefined && (
          <button class="game-secondary-button" type="button" onClick={onRetry}>
            {t('games.action.retry')}
          </button>
        )}
      </div>

      {readiness !== null && readiness.blockers.length > 0 && (
        <ul class="game-blockers">
          {readiness.blockers.map((blocker) => {
            const copy = blockerCopy(blocker);
            return (
              <li key={blocker.code}>
                <div class="game-blockers__icon" aria-hidden="true">
                  {blocker.status === 'ready' ? '✓' : '·'}
                </div>
                <div>
                  <strong>{copy.title}</strong>
                  <p>{copy.detail}</p>
                </div>
                <span class={`game-gate game-gate--${blocker.status}`}>
                  {t(`games.blocker.status.${blocker.status}` as const)}
                </span>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

function formatPlayers(instance: GameInstanceSummary): string {
  return instance.playersMax === null
    ? String(instance.playersOnline)
    : `${instance.playersOnline} / ${instance.playersMax}`;
}

function formatMemory(instance: GameInstanceSummary): string {
  if (instance.memoryUsedBytes === null) return t('games.instance.notReported');
  if (instance.memoryLimitBytes === null) return formatBytes(instance.memoryUsedBytes);
  return `${formatBytes(instance.memoryUsedBytes)} / ${formatBytes(instance.memoryLimitBytes)}`;
}

function GameCard({ instance }: { instance: GameInstanceSummary }) {
  const tone = statusTone(instance.status);
  return (
    <article class={`game-card game-card--${tone}`}>
      <div class="game-card__header">
        <span class="game-card__mark" aria-hidden="true">
          {instance.game.slice(0, 2).toLocaleUpperCase()}
        </span>
        <div class="game-card__identity">
          <span>{instance.game} · {instance.software} {instance.version}</span>
          <h3>{instance.name}</h3>
        </div>
        <GameStatus status={instance.status} />
      </div>
      <dl class="game-card__metrics">
        <div><dt>{t('games.instance.players')}</dt><dd>{formatPlayers(instance)}</dd></div>
        <div><dt>{t('games.instance.cpu')}</dt><dd>{instance.cpuPercent === null ? t('games.instance.notReported') : formatPercent(instance.cpuPercent)}</dd></div>
        <div><dt>{t('games.instance.memory')}</dt><dd>{formatMemory(instance)}</dd></div>
        <div><dt>{t('games.instance.uptime')}</dt><dd>{instance.uptimeSeconds === null ? t('games.instance.notReported') : formatDuration(instance.uptimeSeconds)}</dd></div>
      </dl>
      <div class="game-card__facts">
        <div><span>{t('games.instance.address')}</span><code>{instance.address ?? t('games.instance.notReported')}</code></div>
        <div><span>{t('games.instance.update')}</span><strong>{t(updateLabelIds[instance.updateStatus])}</strong></div>
        <div><span>{t('games.instance.backup')}</span><strong>{t(backupLabelIds[instance.backupStatus])}</strong></div>
      </div>
      <div class="game-card__footer">
        <span class={instance.warnings.length > 0 ? 'game-warning-count' : 'game-warning-count game-warning-count--clear'}>
          {instance.warnings.length > 0 ? t('games.instance.warnings', instance.warnings.length) : t('games.instance.noWarnings')}
        </span>
        <a class="game-open-link" href={`#games/${encodeURIComponent(instance.id)}`}>
          {t('games.instance.open', instance.name)} <span aria-hidden="true">→</span>
        </a>
      </div>
    </article>
  );
}

function GameCompactRow({ instance }: { instance: GameInstanceSummary }) {
  return (
    <li class="game-row">
      <div class="game-row__identity">
        <span class="game-card__mark" aria-hidden="true">{instance.game.slice(0, 2).toLocaleUpperCase()}</span>
        <div><strong>{instance.name}</strong><span>{instance.game} · {instance.version}</span></div>
      </div>
      <GameStatus status={instance.status} />
      <span><small>{t('games.instance.players')}</small>{formatPlayers(instance)}</span>
      <span><small>{t('games.instance.memory')}</small>{formatMemory(instance)}</span>
      <span><small>{t('games.instance.address')}</small><code>{instance.address ?? '—'}</code></span>
      <a class="game-open-link" href={`#games/${encodeURIComponent(instance.id)}`} aria-label={t('games.instance.open', instance.name)}>→</a>
    </li>
  );
}

function ServerRegistry({
  instances,
  totalInstances,
}: {
  instances: readonly GameInstanceSummary[] | null;
  totalInstances: number;
}) {
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<GameFilter>('all');
  const [viewMode, setViewMode] = useState<GameViewMode>('cards');
  const filtered = useMemo(
    () => (instances === null ? [] : filterGameInstances(instances, query, filter)),
    [instances, query, filter],
  );

  if (instances === null) {
    return (
      <section class="game-registry game-registry--locked" aria-labelledby="game-registry-title">
        <span class="eyebrow">{t('games.registry.eyebrow')}</span>
        <div class="game-registry__locked-mark" aria-hidden="true">⌁</div>
        <h2 id="game-registry-title">{t('games.registry.lockedTitle')}</h2>
        <p>{t('games.registry.lockedDetail')}</p>
        <button class="game-primary-button" type="button" disabled title={t('games.action.newServerUnavailable')}>
          {t('games.action.newServer')}
        </button>
      </section>
    );
  }

  return (
    <section class="game-registry" aria-labelledby="game-registry-title">
      <div class="game-registry__heading">
        <div>
          <span class="eyebrow">{t('games.registry.eyebrow')}</span>
          <h2 id="game-registry-title">{t('games.registry.readyTitle')}</h2>
          <p>{t('games.registry.count', instances.length, totalInstances)}</p>
        </div>
      </div>

      {instances.length > 0 && (
        <div class="game-toolbar">
          <label class="game-search">
            <span>{t('games.filter.search')}</span>
            <input
              type="search"
              value={query}
              placeholder={t('games.filter.searchPlaceholder')}
              onInput={(event) => setQuery(event.currentTarget.value)}
            />
          </label>
          <label class="game-filter">
            <span>{t('games.filter.status')}</span>
            <select value={filter} onChange={(event) => setFilter(event.currentTarget.value as GameFilter)}>
              <option value="all">{t('games.filter.all')}</option>
              <option value="online">{t('games.filter.online')}</option>
              <option value="offline">{t('games.filter.offline')}</option>
              <option value="attention">{t('games.filter.attention')}</option>
            </select>
          </label>
          <div class="game-view-toggle" aria-label={t('games.view.label')}>
            <button type="button" class={viewMode === 'cards' ? 'is-active' : undefined} aria-pressed={viewMode === 'cards'} onClick={() => setViewMode('cards')}>
              {t('games.view.cards')}
            </button>
            <button type="button" class={viewMode === 'compact' ? 'is-active' : undefined} aria-pressed={viewMode === 'compact'} onClick={() => setViewMode('compact')}>
              {t('games.view.list')}
            </button>
          </div>
        </div>
      )}

      {filtered.length === 0 ? (
        <div class="game-empty-state">
          <strong>{instances.length === 0 ? t('games.empty.title') : t('games.empty.filteredTitle')}</strong>
          <span>{instances.length === 0 ? t('games.empty.detail') : t('games.empty.filteredDetail')}</span>
        </div>
      ) : viewMode === 'cards' ? (
        <div class="game-card-grid">{filtered.map((instance) => <GameCard key={instance.id} instance={instance} />)}</div>
      ) : (
        <ul class="game-compact-list">{filtered.map((instance) => <GameCompactRow key={instance.id} instance={instance} />)}</ul>
      )}
    </section>
  );
}

function InstanceDetail({ instance }: { instance: GameInstanceSummary }) {
  const [selectedTab, setSelectedTab] = useState<'overview' | GameInstanceFeature>('overview');
  const tabs: Array<'overview' | GameInstanceFeature> = ['overview', ...instance.features];
  const selectAdjacentTab = (direction: -1 | 1): void => {
    const current = tabs.indexOf(selectedTab);
    const next = (current + direction + tabs.length) % tabs.length;
    const nextTab = tabs[next]!;
    setSelectedTab(nextTab);
    window.requestAnimationFrame(() => document.querySelector<HTMLButtonElement>(
      `[data-game-tab="${nextTab}"]`,
    )?.focus());
  };
  return (
    <section class="game-detail" aria-labelledby="game-detail-title">
      <a class="game-back-link" href="#games">← {t('games.detail.back')}</a>
      <div class="game-detail__heading">
        <div class="game-detail__identity">
          <span class="game-card__mark" aria-hidden="true">{instance.game.slice(0, 2).toLocaleUpperCase()}</span>
          <div>
            <span class="eyebrow">{t('games.detail.server')} · {instance.game}</span>
            <h1 id="game-detail-title" ref={focusOnMount} tabIndex={-1}>{instance.name}</h1>
            <p>{instance.software} {instance.version} · <code>{instance.address ?? t('games.instance.notReported')}</code></p>
          </div>
        </div>
        <div class="game-detail__actions">
          <GameStatus status={instance.status} />
          {[t('games.detail.start'), t('games.detail.stop'), t('games.detail.restart'), t('games.detail.backup')].map((label) => (
            <button key={label} type="button" disabled title={t('games.action.unavailable')}>{label}</button>
          ))}
        </div>
      </div>

      <GameSummary instances={[instance]} />

      <div
        class="game-tabs"
        role="tablist"
        aria-label={instance.name}
        onKeyDown={(event) => {
          if (event.key === 'ArrowRight') {
            event.preventDefault();
            selectAdjacentTab(1);
          } else if (event.key === 'ArrowLeft') {
            event.preventDefault();
            selectAdjacentTab(-1);
          }
        }}
      >
        {tabs.map((tab) => {
          const label = tab === 'overview' ? t('games.tab.overview') : t(featureLabelIds[tab]);
          return (
            <button
              key={tab}
              id={`game-tab-${tab}`}
              data-game-tab={tab}
              type="button"
              role="tab"
              aria-controls="game-tabpanel"
              aria-selected={selectedTab === tab}
              tabIndex={selectedTab === tab ? 0 : -1}
              class={selectedTab === tab ? 'is-active' : undefined}
              onClick={() => setSelectedTab(tab)}
            >
              {label}
            </button>
          );
        })}
      </div>

      <div
        id="game-tabpanel"
        class="game-detail__content"
        role="tabpanel"
        aria-labelledby={`game-tab-${selectedTab}`}
        tabIndex={0}
      >
        {selectedTab === 'overview' ? (
          <div class="game-detail__panels">
            <section>
              <span class="eyebrow">{t('games.detail.runtime')}</span>
              <h2>{t('games.detail.currentState')}</h2>
              <dl>
                <div><dt>{t('games.detail.software')}</dt><dd>{instance.software}</dd></div>
                <div><dt>{t('games.detail.version')}</dt><dd>{instance.version}</dd></div>
                <div><dt>{t('games.instance.cpu')}</dt><dd>{instance.cpuPercent === null ? '—' : formatPercent(instance.cpuPercent)}</dd></div>
                <div><dt>{t('games.instance.memory')}</dt><dd>{formatMemory(instance)}</dd></div>
              </dl>
            </section>
            <section>
              <span class="eyebrow">{t('games.detail.safety')}</span>
              <h2>{t('games.instance.backup')}</h2>
              <dl>
                <div><dt>{t('games.instance.backup')}</dt><dd>{t(backupLabelIds[instance.backupStatus])}</dd></div>
                <div><dt>{t('games.instance.update')}</dt><dd>{t(updateLabelIds[instance.updateStatus])}</dd></div>
                <div><dt>{t('games.instance.warnings', instance.warnings.length)}</dt><dd>{instance.warnings.length === 0 ? t('games.detail.none') : instance.warnings.join(' · ')}</dd></div>
              </dl>
            </section>
          </div>
        ) : (
          <div class="game-feature-boundary">
            <span class="eyebrow">{t(featureLabelIds[selectedTab])}</span>
            <strong>{t('games.action.unavailable')}</strong>
          </div>
        )}
      </div>
    </section>
  );
}

export function GameWorkspaceView({
  readiness,
  readinessPhase,
  readinessError = null,
  instances,
  totalInstances = instances?.length ?? 0,
  onRetry,
}: GameWorkspaceViewProps) {
  const routeInstanceId = useGameRoute();
  const selectedInstance = instances?.find((instance) => instance.id === routeInstanceId) ?? null;
  if (selectedInstance !== null) return <InstanceDetail instance={selectedInstance} />;

  return (
    <div class="games-workspace">
      <section class="games-hero" aria-labelledby="games-title">
        <div>
          <span class="eyebrow">{t('games.hero.eyebrow')}</span>
          <h1 id="games-title" ref={focusOnMount} tabIndex={-1}>{t('games.hero.title')}</h1>
          <p>{t('games.hero.detail')}</p>
        </div>
        <div class="games-hero__actions">
          {onRetry !== undefined && (
            <button class="game-secondary-button" type="button" onClick={onRetry} disabled={readinessPhase === 'loading'}>
              {readinessPhase === 'loading' ? t('games.action.refreshing') : t('games.action.refresh')}
            </button>
          )}
          <button class="game-primary-button" type="button" disabled title={t('games.action.newServerUnavailable')}>
            <span aria-hidden="true">＋</span> {t('games.action.newServer')}
          </button>
        </div>
      </section>
      <GameSummary instances={instances} />
      <ReadinessPanel
        readiness={readiness}
        phase={readinessPhase}
        error={readinessError}
        {...(onRetry === undefined ? {} : { onRetry })}
      />
      <ServerRegistry instances={instances} totalInstances={totalInstances} />
    </div>
  );
}

export function GamesWorkspace({ csrfToken, onSessionExpired }: GamesWorkspaceProps) {
  const [state, setState] = useState<ReadinessState>({
    data: null,
    phase: 'loading',
    error: null,
  });
  const controller = useRef<AbortController | null>(null);

  const load = useCallback(async (): Promise<void> => {
    controller.current?.abort();
    const nextController = new AbortController();
    controller.current = nextController;
    setState((current) => ({ ...current, phase: 'loading', error: null }));
    try {
      const readiness = await getGameHostingReadiness(csrfToken, nextController.signal);
      if (!nextController.signal.aborted) {
        setState({ data: readiness, phase: 'ready', error: null });
      }
    } catch (error) {
      if (nextController.signal.aborted) return;
      if (error instanceof ApiError && (error.status === 401 || error.code === 'csrf_rejected')) {
        onSessionExpired();
        return;
      }
      setState((current) => ({ ...current, phase: 'error', error: describeError(error) }));
    }
  }, [csrfToken, onSessionExpired]);

  useEffect(() => {
    void load();
    return () => controller.current?.abort();
  }, [load]);

  return (
    <GameWorkspaceView
      readiness={state.data}
      readinessPhase={state.phase}
      readinessError={state.error}
      instances={null}
      onRetry={() => void load()}
    />
  );
}
