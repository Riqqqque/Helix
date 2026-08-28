import { h } from 'preact';
import render from 'preact-render-to-string';
import { describe, expect, it, vi } from 'vitest';
import { DashboardSettingsPage, validateHostReboot, validateRecurringHostReboot } from './dashboard-settings';
import type { HostIntegration, HostRebootPreflight } from './host-api';

const clearPreflight: HostRebootPreflight = {
  canSchedule: true,
  activePlayers: 0,
  activeServerCount: 0,
  activeServers: [],
  activeServersTruncated: false,
  activeJobsTotal: 0,
  activeJobs: [],
  blockers: [],
  checkedAtUnixMs: 1_800_000_000_000,
};

describe('whole-host reboot confirmation', () => {
  it('requires a clear preflight, exact hostname, acknowledgement, and bounded delay', () => {
    expect(validateHostReboot('helix-host', 'helix-host', true, 30, null)).toMatch(/safety check/i);
    expect(validateHostReboot('helix-host', 'helix-host', true, 30, { ...clearPreflight, canSchedule: false })).toMatch(/blocker/i);
    expect(validateHostReboot('helix-host', 'HELIX-HOST', true, 30, clearPreflight)).toMatch(/exactly/i);
    expect(validateHostReboot('helix-host', 'helix-host', false, 30, clearPreflight)).toMatch(/acknowledge/i);
    expect(validateHostReboot('helix-host', 'helix-host', true, 9, clearPreflight)).toMatch(/10 to 300/i);
    expect(validateHostReboot('helix-host', 'helix-host', true, 301, clearPreflight)).toMatch(/10 to 300/i);
    expect(validateHostReboot('helix-host', 'helix-host', true, 30, clearPreflight)).toBeNull();
  });

  it('validates recurring schedules against the verified host timezone and hostname', () => {
    const integration = { hostname: 'helix-host', timezone: 'America/Denver' } as HostIntegration;
    expect(validateRecurringHostReboot(integration, [], '05:00', 'helix-host', true)).toMatch(/weekday/i);
    expect(validateRecurringHostReboot(integration, ['monday'], '25:00', 'helix-host', true)).toMatch(/valid host time/i);
    expect(validateRecurringHostReboot(integration, ['monday'], '05:00', 'wrong', true)).toMatch(/exactly/i);
    expect(validateRecurringHostReboot(integration, ['monday'], '05:00', 'helix-host', false)).toMatch(/acknowledge/i);
    expect(validateRecurringHostReboot({ ...integration, timezone: null }, ['monday'], '05:00', 'helix-host', true)).toMatch(/timezone/i);
    expect(validateRecurringHostReboot(integration, ['monday'], '05:00', 'helix-host', true)).toBeNull();
  });

  it('renders host integration, owner controls, and the uppercase OLED label', () => {
    const markup = render(h(DashboardSettingsPage, {
      user: { id: '019c7714-3b77-44d1-9866-e1f484aae2ab', loginName: 'rique.owner', displayName: 'Rique', capabilities: ['system.view'] },
      csrfToken: 'csrf',
      theme: 'oled',
      refreshIntervalMs: 1_000,
      navigationOrder: ['overview', 'home', 'storage', 'network', 'host', 'security', 'terminal', 'servers', 'hooks'],
      colors: { accent: '', text: '', surface: '' },
      serversEnabled: true,
      preferenceSyncStatus: 'synced',
      hostIntegration: { data: null, phase: 'loading', error: null },
      servers: [],
      onThemeChange: vi.fn(),
      onRefreshIntervalChange: vi.fn(),
      onNavigationOrderChange: vi.fn(),
      onColorsChange: vi.fn(),
      onServersEnabledChange: vi.fn(),
      onAccountUpdated: vi.fn(),
      onHostIntegrationRefresh: vi.fn(async () => undefined),
    }));
    expect(markup).toContain('>OLED<');
    expect(markup).toContain('Host integration');
    expect(markup).toContain('Whole-host reboot');
    expect(markup).toContain('Game servers');
    expect(markup).toContain('Helix data');
    expect(markup).toContain('Save account changes');
    expect(markup).toContain('Current password');
  });
});
