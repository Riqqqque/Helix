import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { ApiError } from './api';
import {
  defaultDashboardColors,
  primaryDashboardSections,
  readDashboardColors,
  readNavigationOrder,
  readRefreshInterval,
  saveDashboardColors,
  saveNavigationOrder,
  saveRefreshInterval,
  saveServersEnabled,
  readServersEnabled,
  type DashboardColors,
  type PrimaryDashboardSectionId,
  type RefreshIntervalMs,
} from './dashboard-preferences';
import {
  defaultHomeTemplates,
  defaultHomeWidgets,
  readActiveHomeId,
  readHomeTemplates,
  saveActiveHomeId,
  saveHomeTemplates,
  saveHomeWidgets,
  type HomeTemplate,
  type HomeWidget,
} from './home-layout';
import {
  getDashboardPreferences,
  putDashboardPreferences,
  type DashboardPreferences,
  type DashboardPreferencesRecord,
} from './preferences-api';

export type PreferenceSyncStatus = 'loading' | 'synced' | 'saving' | 'local';
type PreferenceKey = keyof DashboardPreferences;

interface DashboardPreferenceState extends DashboardPreferences {
  syncStatus: PreferenceSyncStatus;
  setNavigationOrder: (value: PrimaryDashboardSectionId[]) => void;
  setMetricsRefreshMs: (value: RefreshIntervalMs) => void;
  setHomeWidgets: (value: HomeWidget[]) => void;
  setHomeTemplates: (value: HomeTemplate[]) => void;
  setActiveHomeId: (value: string) => void;
  setHomeState: (templates: HomeTemplate[], activeHomeId: string) => void;
  setColors: (value: DashboardColors) => void;
  setServersEnabled: (value: boolean) => void;
}

const SAVE_DEBOUNCE_MS = 700;
const RETRY_DELAY_MS = 30_000;

function sameValue(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

export function shouldMigrateLocalPreferences(
  remoteRevision: number,
  preferences: DashboardPreferences,
): boolean {
  if (remoteRevision !== 0) return false;
  return (
    !sameValue(preferences.navigationOrder, primaryDashboardSections) ||
    preferences.metricsRefreshMs !== 1_000 ||
    !sameValue(preferences.homeWidgets, defaultHomeWidgets) ||
    !sameValue(preferences.homeTemplates, defaultHomeTemplates) ||
    preferences.activeHomeId !== defaultHomeTemplates[0]?.id ||
    !sameValue(preferences.colors, defaultDashboardColors) ||
    preferences.serversEnabled !== true
  );
}

function cachePreferences(preferences: DashboardPreferences): void {
  saveNavigationOrder(preferences.navigationOrder);
  saveRefreshInterval(preferences.metricsRefreshMs);
  saveHomeWidgets(preferences.homeWidgets);
  saveHomeTemplates(preferences.homeTemplates);
  saveActiveHomeId(preferences.activeHomeId, preferences.homeTemplates);
  saveDashboardColors(preferences.colors);
  saveServersEnabled(preferences.serversEnabled);
}

export function mergePreferenceChanges(
  remote: DashboardPreferences,
  local: DashboardPreferences,
  dirty: ReadonlySet<PreferenceKey>,
): DashboardPreferences {
  const homeDirty = dirty.has('homeWidgets') || dirty.has('homeTemplates') || dirty.has('activeHomeId');
  return {
    navigationOrder: dirty.has('navigationOrder') ? local.navigationOrder : remote.navigationOrder,
    metricsRefreshMs: dirty.has('metricsRefreshMs') ? local.metricsRefreshMs : remote.metricsRefreshMs,
    homeWidgets: homeDirty ? local.homeWidgets : remote.homeWidgets,
    homeTemplates: homeDirty ? local.homeTemplates : remote.homeTemplates,
    activeHomeId: homeDirty ? local.activeHomeId : remote.activeHomeId,
    colors: dirty.has('colors') ? local.colors : remote.colors,
    serversEnabled: dirty.has('serversEnabled') ? local.serversEnabled : remote.serversEnabled,
  };
}

export function useDashboardPreferences(
  csrfToken: string,
  onSessionExpired: () => void,
): DashboardPreferenceState {
  const localTemplates = useRef(readHomeTemplates());
  const localActiveHomeId = useRef(readActiveHomeId(localTemplates.current));
  const localInitial = useRef<DashboardPreferences>({
    navigationOrder: readNavigationOrder(),
    metricsRefreshMs: readRefreshInterval(),
    homeTemplates: localTemplates.current,
    activeHomeId: localActiveHomeId.current,
    homeWidgets: localTemplates.current.find((template) => template.id === localActiveHomeId.current)?.widgets ?? [],
    colors: readDashboardColors(),
    serversEnabled: readServersEnabled(),
  });
  const [preferences, setPreferences] = useState<DashboardPreferences>(localInitial.current);
  const [syncStatus, setSyncStatus] = useState<PreferenceSyncStatus>('loading');
  const [loadAttempt, setLoadAttempt] = useState(0);
  const [syncTick, setSyncTick] = useState(0);
  const stateRef = useRef(preferences);
  const revisionRef = useRef<number | null>(null);
  const initializedRef = useRef(false);
  const mountedRef = useRef(true);
  const inFlightRef = useRef(false);
  const dirtyRef = useRef(new Set<PreferenceKey>());
  const generationsRef = useRef<Record<PreferenceKey, number>>({
    navigationOrder: 0,
    metricsRefreshMs: 0,
    homeWidgets: 0,
    homeTemplates: 0,
    activeHomeId: 0,
    colors: 0,
    serversEnabled: 0,
  });
  const retryTimerRef = useRef<number | null>(null);

  const commitLocal = useCallback((next: DashboardPreferences): void => {
    stateRef.current = next;
    setPreferences(next);
    cachePreferences(next);
  }, []);

  const change = useCallback(<Key extends PreferenceKey>(
    key: Key,
    value: DashboardPreferences[Key],
  ): void => {
    const next = { ...stateRef.current, [key]: value };
    dirtyRef.current.add(key);
    generationsRef.current[key] += 1;
    commitLocal(next);
    setSyncStatus(revisionRef.current === null ? 'local' : 'saving');
    setSyncTick((current) => current + 1);
  }, [commitLocal]);

  const changeHome = useCallback((homeTemplates: HomeTemplate[], activeHomeId: string): void => {
    const selected = homeTemplates.find((template) => template.id === activeHomeId) ?? homeTemplates[0];
    if (selected === undefined) return;
    const next = {
      ...stateRef.current,
      homeTemplates,
      activeHomeId: selected.id,
      homeWidgets: selected.widgets,
    };
    for (const key of ['homeWidgets', 'homeTemplates', 'activeHomeId'] as const) {
      dirtyRef.current.add(key);
      generationsRef.current[key] += 1;
    }
    commitLocal(next);
    setSyncStatus(revisionRef.current === null ? 'local' : 'saving');
    setSyncTick((current) => current + 1);
  }, [commitLocal]);

  useEffect(() => {
    mountedRef.current = true;
    const controller = new AbortController();
    const load = async (): Promise<void> => {
      try {
        const record = await getDashboardPreferences(csrfToken, controller.signal);
        if (!mountedRef.current || controller.signal.aborted) return;
        revisionRef.current = record.revision;
        initializedRef.current = true;

        if (shouldMigrateLocalPreferences(record.revision, stateRef.current)) {
          dirtyRef.current.add('navigationOrder');
          dirtyRef.current.add('metricsRefreshMs');
          dirtyRef.current.add('homeWidgets');
          dirtyRef.current.add('homeTemplates');
          dirtyRef.current.add('activeHomeId');
          dirtyRef.current.add('colors');
          dirtyRef.current.add('serversEnabled');
          setSyncStatus('saving');
          setSyncTick((current) => current + 1);
          return;
        }

        const next = mergePreferenceChanges(record.preferences, stateRef.current, dirtyRef.current);
        commitLocal(next);
        setSyncStatus(dirtyRef.current.size === 0 ? 'synced' : 'saving');
        if (dirtyRef.current.size > 0) setSyncTick((current) => current + 1);
      } catch (error) {
        if (!mountedRef.current || controller.signal.aborted) return;
        if (error instanceof ApiError && error.status === 401) {
          onSessionExpired();
          return;
        }
        initializedRef.current = true;
        setSyncStatus('local');
        retryTimerRef.current = window.setTimeout(
          () => setLoadAttempt((current) => current + 1),
          RETRY_DELAY_MS,
        );
      }
    };
    void load();
    return () => {
      controller.abort();
      if (retryTimerRef.current !== null) window.clearTimeout(retryTimerRef.current);
    };
  }, [commitLocal, csrfToken, loadAttempt, onSessionExpired]);

  useEffect(() => {
    if (!initializedRef.current || revisionRef.current === null || dirtyRef.current.size === 0) return;
    const timer = window.setTimeout(() => {
      const flush = async (): Promise<void> => {
        if (inFlightRef.current || revisionRef.current === null) {
          setSyncTick((current) => current + 1);
          return;
        }
        inFlightRef.current = true;
        setSyncStatus('saving');
        const dirty = new Set(dirtyRef.current);
        const generations = { ...generationsRef.current };
        const desired = stateRef.current;
        let accepted: DashboardPreferencesRecord;
        try {
          try {
            accepted = await putDashboardPreferences(revisionRef.current, desired, csrfToken);
          } catch (error) {
            if (!(error instanceof ApiError) || error.status !== 409) throw error;
            const current = await getDashboardPreferences(csrfToken);
            revisionRef.current = current.revision;
            accepted = await putDashboardPreferences(
              current.revision,
              mergePreferenceChanges(current.preferences, desired, dirty),
              csrfToken,
            );
          }
          if (!mountedRef.current) return;
          revisionRef.current = accepted.revision;
          for (const key of dirty) {
            if (generationsRef.current[key] === generations[key]) dirtyRef.current.delete(key);
          }
          const next = mergePreferenceChanges(accepted.preferences, stateRef.current, dirtyRef.current);
          commitLocal(next);
          setSyncStatus(dirtyRef.current.size === 0 ? 'synced' : 'saving');
          if (dirtyRef.current.size > 0) setSyncTick((current) => current + 1);
        } catch (error) {
          if (!mountedRef.current) return;
          if (error instanceof ApiError && error.status === 401) {
            onSessionExpired();
          } else {
            setSyncStatus('local');
            retryTimerRef.current = window.setTimeout(
              () => setSyncTick((current) => current + 1),
              RETRY_DELAY_MS,
            );
          }
        } finally {
          inFlightRef.current = false;
        }
      };
      void flush();
    }, SAVE_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [commitLocal, csrfToken, onSessionExpired, syncTick]);

  useEffect(() => () => {
    mountedRef.current = false;
    if (retryTimerRef.current !== null) window.clearTimeout(retryTimerRef.current);
  }, []);

  return {
    ...preferences,
    syncStatus,
    setNavigationOrder: (value) => change('navigationOrder', value),
    setMetricsRefreshMs: (value) => change('metricsRefreshMs', value),
    setHomeWidgets: (value) => changeHome(
      preferences.homeTemplates.map((template) => template.id === preferences.activeHomeId ? { ...template, widgets: value } : template),
      preferences.activeHomeId,
    ),
    setHomeTemplates: (value) => changeHome(value, preferences.activeHomeId),
    setActiveHomeId: (value) => changeHome(preferences.homeTemplates, value),
    setHomeState: changeHome,
    setColors: (value) => change('colors', value),
    setServersEnabled: (value: boolean) => change('serversEnabled', value),
  };
}
