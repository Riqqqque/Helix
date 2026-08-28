export const dashboardSectionIds = [
  'overview',
  'home',
  'storage',
  'network',
  'host',
  'security',
  'terminal',
  'servers',
  'hooks',
  'strands',
  'settings',
] as const;

export type DashboardSectionId = (typeof dashboardSectionIds)[number];

export function dashboardSectionForHash(hash: string): DashboardSectionId {
  const candidate = hash.startsWith('#') ? hash.slice(1) : hash;
  if (candidate === '') return 'home';
  if (
    candidate === 'servers' ||
    candidate.startsWith('servers/') ||
    candidate === 'games' ||
    candidate.startsWith('games/')
  ) {
    return 'servers';
  }
  if (candidate === 'strands' || candidate.startsWith('strands/')) {
    return 'strands';
  }
  return dashboardSectionIds.find((id) => id === candidate) ?? 'home';
}
