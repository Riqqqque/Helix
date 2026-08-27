import type { DashboardSectionId } from './navigation';

export const primaryDashboardSections = [
  'overview',
  'home',
  'storage',
  'network',
  'host',
  'terminal',
  'servers',
  'hooks',
] as const satisfies ReadonlyArray<DashboardSectionId>;

export type PrimaryDashboardSectionId = (typeof primaryDashboardSections)[number];

export const refreshIntervalOptions = [1_000, 2_000, 5_000, 10_000, 30_000] as const;
export type RefreshIntervalMs = (typeof refreshIntervalOptions)[number];

export interface DashboardColors {
  accent: string;
  text: string;
  surface: string;
}

export const defaultDashboardColors: DashboardColors = { accent: '', text: '', surface: '' };

const NAVIGATION_STORAGE_KEY = 'helix.dashboard.navigation';
const REFRESH_STORAGE_KEY = 'helix.dashboard.refresh-interval';
const COLORS_STORAGE_KEY = 'helix.dashboard.colors';

export const DASHBOARD_PREFERENCES_EVENT = 'helix:dashboard-preferences';

function canStore(): boolean {
  return typeof globalThis.localStorage !== 'undefined';
}

function announcePreferenceChange(): void {
  if (typeof globalThis.dispatchEvent === 'function' && typeof CustomEvent !== 'undefined') {
    globalThis.dispatchEvent(new CustomEvent(DASHBOARD_PREFERENCES_EVENT));
  }
}

export function normalizeNavigationOrder(value: unknown): PrimaryDashboardSectionId[] {
  if (!Array.isArray(value)) return [...primaryDashboardSections];

  const allowed = new Set<string>(primaryDashboardSections);
  const seen = new Set<string>();
  const order = value.filter((item): item is PrimaryDashboardSectionId => {
    if (typeof item !== 'string' || !allowed.has(item) || seen.has(item)) return false;
    seen.add(item);
    return true;
  });

  for (const section of primaryDashboardSections) {
    if (!seen.has(section)) order.push(section);
  }
  return order;
}

export function readNavigationOrder(): PrimaryDashboardSectionId[] {
  try {
    if (!canStore()) return [...primaryDashboardSections];
    const stored = globalThis.localStorage.getItem(NAVIGATION_STORAGE_KEY);
    return normalizeNavigationOrder(stored === null ? null : JSON.parse(stored));
  } catch {
    return [...primaryDashboardSections];
  }
}

export function saveNavigationOrder(order: readonly PrimaryDashboardSectionId[]): void {
  try {
    if (canStore()) {
      globalThis.localStorage.setItem(
        NAVIGATION_STORAGE_KEY,
        JSON.stringify(normalizeNavigationOrder(order)),
      );
    }
  } catch {
    // Browser storage is a convenience; the dashboard remains usable without it.
  }
  announcePreferenceChange();
}

export function normalizeRefreshInterval(value: unknown): RefreshIntervalMs {
  const candidate = typeof value === 'string' ? Number(value) : value;
  return refreshIntervalOptions.includes(candidate as RefreshIntervalMs)
    ? (candidate as RefreshIntervalMs)
    : 1_000;
}

export function readRefreshInterval(): RefreshIntervalMs {
  try {
    return normalizeRefreshInterval(
      canStore() ? globalThis.localStorage.getItem(REFRESH_STORAGE_KEY) : null,
    );
  } catch {
    return 1_000;
  }
}

export function saveRefreshInterval(value: RefreshIntervalMs): void {
  try {
    if (canStore()) globalThis.localStorage.setItem(REFRESH_STORAGE_KEY, String(value));
  } catch {
    // Browser storage is a convenience; the in-memory preference still applies.
  }
  announcePreferenceChange();
}

function normalizeColor(value: unknown): string {
  return typeof value === 'string' && /^#[0-9a-fA-F]{6}$/u.test(value) ? value.toLowerCase() : '';
}

export function normalizeDashboardColors(value: unknown): DashboardColors {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return { ...defaultDashboardColors };
  const record = value as Record<string, unknown>;
  return {
    accent: normalizeColor(record.accent),
    text: normalizeColor(record.text),
    surface: normalizeColor(record.surface),
  };
}

export function readDashboardColors(): DashboardColors {
  try {
    const stored = canStore() ? globalThis.localStorage.getItem(COLORS_STORAGE_KEY) : null;
    return normalizeDashboardColors(stored === null ? null : JSON.parse(stored));
  } catch {
    return { ...defaultDashboardColors };
  }
}

export function saveDashboardColors(value: DashboardColors): void {
  try {
    if (canStore()) globalThis.localStorage.setItem(COLORS_STORAGE_KEY, JSON.stringify(normalizeDashboardColors(value)));
  } catch {
    // Browser storage is only a local fallback for the server-backed preference.
  }
  announcePreferenceChange();
}

export function applyDashboardColors(value: DashboardColors): void {
  if (typeof document === 'undefined') return;
  const colors = normalizeDashboardColors(value);
  const root = document.documentElement.style;
  const entries: Array<[string, string]> = [
    ['--accent', colors.accent],
    ['--text', colors.text],
    ['--surface', colors.surface],
  ];
  for (const [property, color] of entries) {
    if (color.length === 0) root.removeProperty(property);
    else root.setProperty(property, color);
  }
}

export function moveNavigationItem(
  order: readonly PrimaryDashboardSectionId[],
  section: PrimaryDashboardSectionId,
  offset: -1 | 1,
): PrimaryDashboardSectionId[] {
  const normalized = normalizeNavigationOrder(order);
  const index = normalized.indexOf(section);
  const destination = index + offset;
  if (index === -1 || destination < 0 || destination >= normalized.length) return normalized;
  [normalized[index], normalized[destination]] = [normalized[destination]!, normalized[index]!];
  return normalized;
}
