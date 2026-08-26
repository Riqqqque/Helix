export const dashboardSectionIds = [
  'overview',
  'health',
  'host',
  'storage',
  'network',
] as const;

export type DashboardSectionId = (typeof dashboardSectionIds)[number];

export function dashboardSectionForHash(hash: string): DashboardSectionId {
  const candidate = hash.startsWith('#') ? hash.slice(1) : hash;
  return dashboardSectionIds.find((id) => id === candidate) ?? 'overview';
}
