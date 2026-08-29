import { h } from 'preact';
import render from 'preact-render-to-string';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ManagedServer } from './control-api';
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
      navigationOrder: ['overview', 'home', 'storage', 'network', 'host', 'security', 'terminal', 'servers', 'hooks', 'strands', 'globe'],
      hiddenPages: ['globe'],
      colors: { accent: '', text: '', surface: '' },
      serversEnabled: true,
      preferenceSyncStatus: 'synced',
      hostIntegration: { data: null, phase: 'loading', error: null },
      servers: [],
      onThemeChange: vi.fn(),
      onRefreshIntervalChange: vi.fn(),
      onNavigationOrderChange: vi.fn(),
      onHiddenPagesChange: vi.fn(),
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

const nativeServer: ManagedServer = {
  id: 'helix:018f8fcb-b7af-7f13-9f56-0559788b2c56',
  name: 'Helix native smoke Minecraft server',
  instanceName: 'helix-game-018f8fcb-b7af-7f13-9f56-0559788b2c56',
  software: 'Paper',
  version: '1.21.8',
  status: 'offline',
  panelRunning: false,
  startOnBoot: false,
  playersOnline: 0,
  playerCountVerified: true,
  maxPlayers: 20,
  cpuPercent: 0,
  memoryUsedMb: 0,
  memoryLimitMb: 4_096,
  tps: null,
  managerPanelPort: 0,
  panelPort: 0,
  gamePort: 25_565,
  path: '/srv/helix/servers/smoke',
  warnings: [],
  manager: 'helix',
  executionBackend: 'docker',
  appearance: { kind: 'default', revision: 0 },
  kind: 'minecraft',
};

const settingsProps = {
  user: { id: '019c7714-3b77-44d1-9866-e1f484aae2ab', loginName: 'rique.owner', displayName: 'Rique', capabilities: ['system.view', 'games.manage'] as const },
  csrfToken: 'csrf',
  theme: 'oled' as const,
  refreshIntervalMs: 1_000 as const,
  navigationOrder: ['overview', 'home', 'storage', 'network', 'host', 'security', 'terminal', 'servers', 'hooks', 'strands', 'globe'] as const,
  hiddenPages: ['globe'] as const,
  colors: { accent: '', text: '', surface: '' },
  serversEnabled: true,
  preferenceSyncStatus: 'synced' as const,
  hostIntegration: { data: null, phase: 'loading' as const, error: null },
  servers: [] as ManagedServer[],
  onThemeChange: vi.fn(),
  onRefreshIntervalChange: vi.fn(),
  onNavigationOrderChange: vi.fn(),
  onHiddenPagesChange: vi.fn(),
  onColorsChange: vi.fn(),
  onServersEnabledChange: vi.fn(),
  onAccountUpdated: vi.fn(),
  onHostIntegrationRefresh: vi.fn(async () => undefined),
};

describe('Helix data', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('explains start after boot, dismissed notices, and opens that server', () => {
    const values = new Map<string, string>([
      ['helix.dismissed.v1', JSON.stringify({ 'capacity:/': 1, 'storage-space-intro': 1 })],
    ]);
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => {
        values.set(key, value);
      },
      removeItem: (key: string) => {
        values.delete(key);
      },
    });
    vi.stubGlobal('dispatchEvent', () => true);
    const markup = render(h(DashboardSettingsPage, {
      ...settingsProps,
      user: { ...settingsProps.user, capabilities: [...settingsProps.user.capabilities] },
      navigationOrder: [...settingsProps.navigationOrder],
      hiddenPages: [...settingsProps.hiddenPages],
      servers: [nativeServer],
    }));
    expect(markup).toContain('Helix data');
    expect(markup).toContain('Start after the host boots');
    expect(markup).toContain('does not start or stop the server right now');
    expect(markup).toContain('this browser only');
    expect(markup).toContain('Show them again');
    expect(markup).toContain('Full-disk warning for /');
    expect(markup).toContain('Storage space-analyzer intro');
    expect(markup).toContain('Helix native smoke Minecraft server');
    expect(markup).toContain('#servers/helix%3A018f8fcb-b7af-7f13-9f56-0559788b2c56');
    expect(markup).toContain('Open in Servers');
    expect(markup).not.toContain('>Boot<');
    expect(markup).not.toContain('>Manual<');
  });
});
