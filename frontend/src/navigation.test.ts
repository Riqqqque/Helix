import { describe, expect, it } from 'vitest';
import { dashboardSectionForHash } from './navigation';

describe('dashboard section navigation', () => {
  it('opens Home when the URL has no dashboard fragment', () => {
    expect(dashboardSectionForHash('')).toBe('home');
  });

  it('maps a known URL fragment to its dashboard section', () => {
    expect(dashboardSectionForHash('#storage')).toBe('storage');
    expect(dashboardSectionForHash('#security')).toBe('security');
    expect(dashboardSectionForHash('#strands')).toBe('strands');
    expect(dashboardSectionForHash('#strands/b893d568-327d-4b6e-b0b6-0b7a58e0c852')).toBe('strands');
  });

  it('keeps server detail routes inside the Servers workspace', () => {
    expect(dashboardSectionForHash('#servers')).toBe('servers');
    expect(
      dashboardSectionForHash('#servers/018f8fcb-b7af-7f13-9f56-0559788b2c56'),
    ).toBe('servers');
    expect(dashboardSectionForHash('#games')).toBe('servers');
  });

  it('falls back safely for unknown URL fragments', () => {
    expect(dashboardSectionForHash('#unknown')).toBe('home');
    expect(dashboardSectionForHash('#settings')).toBe('settings');
  });
});
