export const dashboardSectionIds = [
  'overview',
  'home',
  'storage',
  'network',
  'host',
  'terminal',
  'servers',
  'hooks',
  'settings',
] as const;

export type DashboardSectionId = (typeof dashboardSectionIds)[number];

export function dashboardSectionForHash(hash: string): DashboardSectionId {
  const candidate = hash.startsWith('#') ? hash.slice(1) : hash;
  if (
    candidate === 'servers' ||
    candidate.startsWith('servers/') ||
    candidate === 'games' ||
    candidate.startsWith('games/')
  ) {
    return 'servers';
  }
  return dashboardSectionIds.find((id) => id === candidate) ?? 'overview';
}
