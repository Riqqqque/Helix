import { ApiError, expectArray, expectNumber, expectRecord, requestJson } from './api';
import {
  defaultHiddenPages,
  normalizeHiddenPages,
  normalizeNavigationOrder,
  normalizeRefreshInterval,
  normalizeDashboardColors,
  primaryDashboardSections,
  type DashboardColors,
  type PrimaryDashboardSectionId,
  type RefreshIntervalMs,
} from './dashboard-preferences';
import {
  normalizeActiveHomeId,
  normalizeHomeTemplates,
  normalizeHomeWidgets,
  type HomeTemplate,
  type HomeWidget,
} from './home-layout';

export interface DashboardPreferences {
  navigationOrder: PrimaryDashboardSectionId[];
  metricsRefreshMs: RefreshIntervalMs;
  homeWidgets: HomeWidget[];
  homeTemplates: HomeTemplate[];
  activeHomeId: string;
  colors: DashboardColors;
  serversEnabled: boolean;
  hiddenPages: PrimaryDashboardSectionId[];
}

export interface DashboardPreferencesRecord {
  revision: number;
  preferences: DashboardPreferences;
  updatedAtUnixMs: number | null;
}

export function normalizeDashboardPreferences(value: DashboardPreferences): DashboardPreferences {
  const homeTemplates = normalizeHomeTemplates(value.homeTemplates, value.homeWidgets);
  const activeHomeId = normalizeActiveHomeId(value.activeHomeId, homeTemplates);
  const activeWidgets = homeTemplates.find((template) => template.id === activeHomeId)?.widgets ?? [];
  return {
    navigationOrder: normalizeNavigationOrder(value.navigationOrder),
    metricsRefreshMs: normalizeRefreshInterval(value.metricsRefreshMs),
    homeWidgets: normalizeHomeWidgets(activeWidgets),
    homeTemplates,
    activeHomeId,
    colors: normalizeDashboardColors(value.colors),
    serversEnabled: value.serversEnabled !== false,
    hiddenPages: normalizeHiddenPages(value.hiddenPages),
  };
}

function parsePreferences(value: unknown): DashboardPreferences {
  const record = expectRecord(value, 'Dashboard preferences');
  const rawOrder = expectArray(
    record,
    'navigationOrder',
    'Dashboard preferences',
    primaryDashboardSections.length,
  );
  const navigationOrder = normalizeNavigationOrder(rawOrder);
  if (
    rawOrder.length !== primaryDashboardSections.length ||
    navigationOrder.length !== rawOrder.length ||
    rawOrder.some((entry, index) => entry !== navigationOrder[index])
  ) throw new ApiError('Dashboard preferences returned an invalid navigationOrder value.');

  const metricsRefreshMs = expectNumber(
    record,
    'metricsRefreshMs',
    'Dashboard preferences',
    { integer: true, minimum: 1_000, maximum: 30_000 },
  );
  if (normalizeRefreshInterval(metricsRefreshMs) !== metricsRefreshMs) {
    throw new ApiError('Dashboard preferences returned an invalid metricsRefreshMs value.');
  }

  const rawWidgets = expectArray(record, 'homeWidgets', 'Dashboard preferences', 32);
  const homeWidgets = normalizeHomeWidgets(rawWidgets);
  if (homeWidgets.length !== rawWidgets.length) {
    throw new ApiError('Dashboard preferences returned an invalid homeWidgets value.');
  }
  const rawTemplates = expectArray(record, 'homeTemplates', 'Dashboard preferences', 8);
  const homeTemplates = normalizeHomeTemplates(rawTemplates, homeWidgets);
  if (homeTemplates.length !== rawTemplates.length) {
    throw new ApiError('Dashboard preferences returned an invalid homeTemplates value.');
  }
  const activeHomeId = typeof record.activeHomeId === 'string'
    ? normalizeActiveHomeId(record.activeHomeId, homeTemplates)
    : '';
  if (activeHomeId !== record.activeHomeId) {
    throw new ApiError('Dashboard preferences returned an invalid activeHomeId value.');
  }
  const active = homeTemplates.find((template) => template.id === activeHomeId);
  if (active === undefined || JSON.stringify(active.widgets) !== JSON.stringify(homeWidgets)) {
    throw new ApiError('Dashboard preferences returned inconsistent active Home widgets.');
  }
  const colors = normalizeDashboardColors(record.colors);
  if (JSON.stringify(colors) !== JSON.stringify(record.colors)) {
    throw new ApiError('Dashboard preferences returned an invalid colors value.');
  }
  if (record.serversEnabled !== undefined && typeof record.serversEnabled !== 'boolean') {
    throw new ApiError('Dashboard preferences returned an invalid serversEnabled value.');
  }
  const hiddenPages = parseHiddenPages(record.hiddenPages);
  return {
    navigationOrder,
    metricsRefreshMs,
    homeWidgets,
    homeTemplates,
    activeHomeId,
    colors,
    serversEnabled: record.serversEnabled !== false,
    hiddenPages,
  };
}

function parseHiddenPages(value: unknown): PrimaryDashboardSectionId[] {
  if (value === undefined) return [...defaultHiddenPages];
  if (!Array.isArray(value) || value.length > primaryDashboardSections.length) {
    throw new ApiError('Dashboard preferences returned an invalid hiddenPages value.');
  }
  const normalized = normalizeHiddenPages(value);
  if (
    normalized.length !== value.length
    || value.some((entry, index) => entry !== normalized[index])
  ) {
    throw new ApiError('Dashboard preferences returned an invalid hiddenPages value.');
  }
  return normalized;
}

export function parseDashboardPreferencesRecord(value: unknown): DashboardPreferencesRecord {
  const record = expectRecord(value, 'Dashboard preferences API');
  const revision = expectNumber(record, 'revision', 'Dashboard preferences API', {
    integer: true,
    minimum: 0,
  });
  const updatedAtUnixMs = record.updatedAtUnixMs === null
    ? null
    : expectNumber(
        record,
        'updatedAtUnixMs',
        'Dashboard preferences API',
        { integer: true, minimum: 0 },
      );
  if (revision > 0 && updatedAtUnixMs === null) {
    throw new ApiError('Dashboard preferences returned a missing update timestamp.');
  }
  return {
    revision,
    preferences: parsePreferences(record.preferences),
    updatedAtUnixMs,
  };
}

export function getDashboardPreferences(
  csrfToken: string,
  signal?: AbortSignal,
): Promise<DashboardPreferencesRecord> {
  return requestJson('/api/v1/settings/preferences', parseDashboardPreferencesRecord, {
    csrfToken,
    signal,
  });
}

export function putDashboardPreferences(
  expectedRevision: number,
  preferences: DashboardPreferences,
  csrfToken: string,
): Promise<DashboardPreferencesRecord> {
  return requestJson('/api/v1/settings/preferences', parseDashboardPreferencesRecord, {
    method: 'PUT',
    body: {
      expectedRevision,
      preferences: normalizeDashboardPreferences(preferences),
    },
    csrfToken,
  });
}
