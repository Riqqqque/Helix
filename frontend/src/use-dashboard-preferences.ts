import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { ApiError } from './api';
import {
  defaultDashboardColors,
  defaultHiddenPages,
  isFactoryRefreshInterval,
  primaryDashboardSections,
  readDashboardColors,
  readHiddenPages,
  readNavigationOrder,
  readRefreshInterval,
  saveDashboardColors,
  saveHiddenPages,
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
  normalizeDashboardPreferences,
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
  setHiddenPages: (value: PrimaryDashboardSectionId[]) => void;
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
    !isFactoryRefreshInterval(preferences.metricsRefreshMs) ||
    !sameValue(preferences.homeWidgets, defaultHomeWidgets) ||
    !sameValue(preferences.homeTemplates, defaultHomeTemplates) ||
    preferences.activeHomeId !== defaultHomeTemplates[0]?.id ||
    !sameValue(preferences.colors, defaultDashboardColors) ||
    preferences.serversEnabled !== true ||
    !sameValue(preferences.hiddenPages, defaultHiddenPages)
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
  saveHiddenPages(preferences.hiddenPages);
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
    hiddenPages: dirty.has('hiddenPages') ? local.hiddenPages : remote.hiddenPages,
  };
}

const UNSYNCED_STORAGE_KEY = 'helix.dashboard.unsynced';
const dashboardPreferenceKeys: readonly PreferenceKey[] = [
  'navigationOrder',
  'metricsRefreshMs',
  'homeWidgets',
  'homeTemplates',
  'activeHomeId',
  'colors',
  'serversEnabled',
  'hiddenPages',
];

export interface UnsyncedDashboardPreferences {
  dirty: PreferenceKey[];
  preferences: DashboardPreferences;
}

function parseUnsyncedDirty(value: unknown): PreferenceKey[] {
  if (!Array.isArray(value)) return [];
  const allowed = new Set<string>(dashboardPreferenceKeys);
  const dirty: PreferenceKey[] = [];
  for (const item of value) {
    if (typeof item !== 'string' || !allowed.has(item) || dirty.includes(item as PreferenceKey)) continue;
    dirty.push(item as PreferenceKey);
  }
  return dirty;
}

export function readUnsyncedDashboardPreferences(): UnsyncedDashboardPreferences | null {
  try {
    const raw = globalThis.localStorage?.getItem(UNSYNCED_STORAGE_KEY);
    if (raw === null || raw === undefined || raw.length === 0) return null;
    const parsed: unknown = JSON.parse(raw);
    if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) return null;
    const record = parsed as Record<string, unknown>;
    const dirty = parseUnsyncedDirty(record.dirty);
    if (dirty.length === 0) return null;
    if (typeof record.preferences !== 'object' || record.preferences === null || Array.isArray(record.preferences)) {
      return null;
    }
    return {
      dirty,
      preferences: normalizeDashboardPreferences(record.preferences as DashboardPreferences),
    };
  } catch {
    return null;
  }
}

export function writeUnsyncedDashboardPreferences(
  dirty: ReadonlySet<PreferenceKey>,
  preferences: DashboardPreferences,
): void {
  try {
    if (dirty.size === 0) {
      globalThis.localStorage?.removeItem(UNSYNCED_STORAGE_KEY);
      return;
    }
    globalThis.localStorage?.setItem(
      UNSYNCED_STORAGE_KEY,
      JSON.stringify({
        dirty: dashboardPreferenceKeys.filter((key) => dirty.has(key)),
        preferences: normalizeDashboardPreferences(preferences),
      }),
    );
  } catch {
    // Browser storage is a convenience; the in-memory copy still applies.
  }
}

export function clearUnsyncedDashboardPreferences(): void {
  try {
    globalThis.localStorage?.removeItem(UNSYNCED_STORAGE_KEY);
  } catch {
    // Ignoring storage failures keeps the dashboard usable.
  }
}

function readStoredDashboardPreferences(): DashboardPreferences {
  const homeTemplates = readHomeTemplates();
  const activeHomeId = readActiveHomeId(homeTemplates);
  return {
    navigationOrder: readNavigationOrder(),
    metricsRefreshMs: readRefreshInterval(),
    homeTemplates,
    activeHomeId,
    homeWidgets: homeTemplates.find((template) => template.id === activeHomeId)?.widgets ?? [],
    colors: readDashboardColors(),
    serversEnabled: readServersEnabled(),
    hiddenPages: readHiddenPages(),
  };
}

export function useDashboardPreferences(
  csrfToken: string,
  onSessionExpired: () => void,
): DashboardPreferenceState {
  const startup = useRef(readUnsyncedDashboardPreferences());
  const localInitial = useRef<DashboardPreferences>(
    startup.current?.preferences ?? readStoredDashboardPreferences(),
  );
  const [preferences, setPreferences] = useState<DashboardPreferences>(localInitial.current);
  const [syncStatus, setSyncStatus] = useState<PreferenceSyncStatus>('loading');
  const [loadAttempt, setLoadAttempt] = useState(0);
  const [syncTick, setSyncTick] = useState(0);
  const stateRef = useRef(preferences);
  const revisionRef = useRef<number | null>(null);
  const initializedRef = useRef(false);
  const mountedRef = useRef(true);
  const inFlightRef = useRef(false);
  const dirtyRef = useRef(new Set<PreferenceKey>(startup.current?.dirty ?? []));
  const generationsRef = useRef<Record<PreferenceKey, number>>({
    navigationOrder: 0,
    metricsRefreshMs: 0,
    homeWidgets: 0,
    homeTemplates: 0,
    activeHomeId: 0,
    colors: 0,
    serversEnabled: 0,
    hiddenPages: 0,
  });
  const retryTimerRef = useRef<number | null>(null);

  const commitLocal = useCallback((next: DashboardPreferences): void => {
    stateRef.current = next;
    setPreferences(next);
    cachePreferences(next);
    if (dirtyRef.current.size === 0) clearUnsyncedDashboardPreferences();
    else writeUnsyncedDashboardPreferences(dirtyRef.current, next);
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
          for (const key of dashboardPreferenceKeys) dirtyRef.current.add(key);
          commitLocal(stateRef.current);
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

  const flushPendingPreferences = useCallback(async (): Promise<void> => {
    if (!initializedRef.current || revisionRef.current === null || dirtyRef.current.size === 0) return;
    if (inFlightRef.current) {
      setSyncTick((current) => current + 1);
      return;
    }
    inFlightRef.current = true;
    setSyncStatus('saving');
    const dirty = new Set(dirtyRef.current);
    const generations = { ...generationsRef.current };
    const desired = stateRef.current;
    try {
      let accepted: DashboardPreferencesRecord;
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
  }, [commitLocal, csrfToken, onSessionExpired]);

  useEffect(() => {
    if (!initializedRef.current || revisionRef.current === null || dirtyRef.current.size === 0) return;
    const timer = window.setTimeout(() => {
      void flushPendingPreferences();
    }, SAVE_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [flushPendingPreferences, syncTick]);

  useEffect(() => {
    if (typeof window === 'undefined') return;
    const flushNow = (): void => {
      void flushPendingPreferences();
    };
    const onVisibility = (): void => {
      if (document.visibilityState === 'hidden') flushNow();
    };
    window.addEventListener('pagehide', flushNow);
    document.addEventListener('visibilitychange', onVisibility);
    return () => {
      window.removeEventListener('pagehide', flushNow);
      document.removeEventListener('visibilitychange', onVisibility);
    };
  }, [flushPendingPreferences]);

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
    setServersEnabled: (value: boolean) => {
      const currentHidden = stateRef.current.hiddenPages;
      const nextHidden: PrimaryDashboardSectionId[] = value
        ? currentHidden.filter((id) => id !== 'servers')
        : currentHidden.includes('servers') ? currentHidden : [...currentHidden, 'servers'];
      const next = { ...stateRef.current, serversEnabled: value, hiddenPages: nextHidden };
      dirtyRef.current.add('serversEnabled');
      generationsRef.current.serversEnabled += 1;
      if (!sameValue(nextHidden, stateRef.current.hiddenPages)) {
        dirtyRef.current.add('hiddenPages');
        generationsRef.current.hiddenPages += 1;
      }
      commitLocal(next);
      setSyncStatus(revisionRef.current === null ? 'local' : 'saving');
      setSyncTick((current) => current + 1);
    },
    setHiddenPages: (value) => change('hiddenPages', value),
  };
}
