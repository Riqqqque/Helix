import { describe, expect, it } from 'vitest';
import { dashboardSectionForHash } from './navigation';

describe('dashboard section navigation', () => {
  it('uses overview when the URL has no dashboard fragment', () => {
    expect(dashboardSectionForHash('')).toBe('overview');
  });

  it('maps a known URL fragment to its dashboard section', () => {
    expect(dashboardSectionForHash('#storage')).toBe('storage');
  });

  it('falls back safely for unknown URL fragments', () => {
    expect(dashboardSectionForHash('#settings')).toBe('overview');
  });
});
