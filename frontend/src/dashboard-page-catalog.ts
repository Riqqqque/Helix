import {
  normalizeHiddenPages,
  normalizeNavigationOrder,
  visibleDashboardSections,
  type PrimaryDashboardSectionId,
} from './dashboard-preferences';

export function addableDashboardSections(
  order: readonly PrimaryDashboardSectionId[],
  hiddenPages: readonly PrimaryDashboardSectionId[],
  serversEnabled: boolean,
): PrimaryDashboardSectionId[] {
  const hidden = new Set(hiddenPages);
  return normalizeNavigationOrder(order).filter(
    (id) => hidden.has(id) || (id === 'servers' && !serversEnabled),
  );
}

export function hideDashboardPage(
  hiddenPages: readonly PrimaryDashboardSectionId[],
  section: PrimaryDashboardSectionId,
): PrimaryDashboardSectionId[] {
  const next = normalizeHiddenPages(hiddenPages);
  if (!next.includes(section)) next.push(section);
  return next;
}

export function showDashboardPage(
  hiddenPages: readonly PrimaryDashboardSectionId[],
  section: PrimaryDashboardSectionId,
): PrimaryDashboardSectionId[] {
  return normalizeHiddenPages(hiddenPages).filter((id) => id !== section);
}

export function moveVisibleNavigationItem(
  order: readonly PrimaryDashboardSectionId[],
  hiddenPages: readonly PrimaryDashboardSectionId[],
  serversEnabled: boolean,
  section: PrimaryDashboardSectionId,
  offset: -1 | 1,
): PrimaryDashboardSectionId[] {
  const normalized = normalizeNavigationOrder(order);
  const visible = visibleDashboardSections(normalized, hiddenPages, serversEnabled);
  const index = visible.indexOf(section);
  const destination = index + offset;
  if (index === -1 || destination < 0 || destination >= visible.length) return normalized;
  const from = normalized.indexOf(section);
  const to = normalized.indexOf(visible[destination]!);
  if (from < 0 || to < 0) return normalized;
  [normalized[from], normalized[to]] = [normalized[to]!, normalized[from]!];
  return normalized;
}
