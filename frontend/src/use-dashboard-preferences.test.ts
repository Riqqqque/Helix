import { afterEach, describe, expect, it, vi } from 'vitest';
import { defaultHomeTemplates, defaultHomeWidgets } from './home-layout';
import type { DashboardPreferences } from './preferences-api';
import {
  clearUnsyncedDashboardPreferences,
  mergePreferenceChanges,
  readUnsyncedDashboardPreferences,
  shouldMigrateLocalPreferences,
  writeUnsyncedDashboardPreferences,
} from './use-dashboard-preferences';

const defaults: DashboardPreferences = {
    navigationOrder: ['overview', 'home', 'storage', 'network', 'host', 'security', 'terminal', 'servers', 'hooks', 'strands', 'globe'],
  metricsRefreshMs: 5_000,
  homeWidgets: defaultHomeWidgets.map((widget) => ({ ...widget })),
  homeTemplates: defaultHomeTemplates.map((template) => ({ ...template, widgets: template.widgets.map((widget) => ({ ...widget })) })),
  activeHomeId: 'home-main',
  colors: { accent: '', text: '', surface: '' },
  serversEnabled: true,
  hiddenPages: ['globe'],
};

afterEach(() => vi.unstubAllGlobals());

describe('dashboard preference reconciliation', () => {
  it('never migrates browser defaults over an established server revision', () => {
    expect(shouldMigrateLocalPreferences(8, {
      ...defaults,
      navigationOrder: ['servers', 'overview', 'home', 'storage', 'network', 'host', 'security', 'terminal', 'hooks', 'strands', 'globe'],
    })).toBe(false);
  });

    it('migrates a customized browser layout only into an unset record', () => {
    const local = { ...defaults, metricsRefreshMs: 2_000 as const };
    expect(shouldMigrateLocalPreferences(0, local)).toBe(true);
    expect(shouldMigrateLocalPreferences(1, local)).toBe(false);
    expect(shouldMigrateLocalPreferences(0, defaults)).toBe(false);
    expect(shouldMigrateLocalPreferences(0, { ...defaults, metricsRefreshMs: 1_000 })).toBe(false);
  });

  it('migrates a disabled Servers module into an unset record', () => {
    expect(shouldMigrateLocalPreferences(0, { ...defaults, serversEnabled: false })).toBe(true);
  });

  it('rebases only fields changed by this browser after a CAS conflict', () => {
    const remote: DashboardPreferences = {
      ...defaults,
      navigationOrder: ['home', 'overview', 'storage', 'network', 'host', 'security', 'terminal', 'servers', 'hooks', 'strands', 'globe'],
    };
    const local: DashboardPreferences = {
      ...defaults,
      metricsRefreshMs: 2_000,
    };
    expect(mergePreferenceChanges(remote, local, new Set(['metricsRefreshMs']))).toEqual({
      ...remote,
      metricsRefreshMs: 2_000,
    });
  });

  it('keeps unsynced page order across a reload before Helix finishes saving', () => {
    const values = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => {
        values.set(key, value);
      },
      removeItem: (key: string) => {
        values.delete(key);
      },
    });
    const local: DashboardPreferences = {
      ...defaults,
      navigationOrder: ['servers', 'overview', 'home', 'storage', 'network', 'host', 'security', 'terminal', 'hooks', 'strands', 'globe'],
    };
    writeUnsyncedDashboardPreferences(new Set(['navigationOrder']), local);
    const pending = readUnsyncedDashboardPreferences();
    expect(pending?.dirty).toEqual(['navigationOrder']);
    expect(pending?.preferences.navigationOrder[0]).toBe('servers');
    expect(
      mergePreferenceChanges(defaults, pending!.preferences, new Set(pending!.dirty)).navigationOrder[0],
    ).toBe('servers');
    writeUnsyncedDashboardPreferences(new Set(), local);
    expect(readUnsyncedDashboardPreferences()).toBeNull();
    writeUnsyncedDashboardPreferences(new Set(['navigationOrder']), local);
    clearUnsyncedDashboardPreferences();
    expect(readUnsyncedDashboardPreferences()).toBeNull();
  });
});
