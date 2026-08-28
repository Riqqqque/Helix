import { describe, expect, it } from 'vitest';
import { defaultHomeTemplates, defaultHomeWidgets } from './home-layout';
import type { DashboardPreferences } from './preferences-api';
import {
  mergePreferenceChanges,
  shouldMigrateLocalPreferences,
} from './use-dashboard-preferences';

const defaults: DashboardPreferences = {
  navigationOrder: ['overview', 'home', 'storage', 'network', 'host', 'terminal', 'servers', 'hooks'],
  metricsRefreshMs: 1_000,
  homeWidgets: defaultHomeWidgets.map((widget) => ({ ...widget })),
  homeTemplates: defaultHomeTemplates.map((template) => ({ ...template, widgets: template.widgets.map((widget) => ({ ...widget })) })),
  activeHomeId: 'home-main',
  colors: { accent: '', text: '', surface: '' },
  serversEnabled: true,
};

describe('dashboard preference reconciliation', () => {
  it('never migrates browser defaults over an established server revision', () => {
    expect(shouldMigrateLocalPreferences(8, {
      ...defaults,
      navigationOrder: ['servers', 'overview', 'home', 'storage', 'network', 'host', 'terminal', 'hooks'],
    })).toBe(false);
  });

    it('migrates a customized browser layout only into an unset record', () => {
    const local = { ...defaults, metricsRefreshMs: 5_000 as const };
    expect(shouldMigrateLocalPreferences(0, local)).toBe(true);
    expect(shouldMigrateLocalPreferences(1, local)).toBe(false);
    expect(shouldMigrateLocalPreferences(0, defaults)).toBe(false);
  });

  it('migrates a disabled Servers module into an unset record', () => {
    expect(shouldMigrateLocalPreferences(0, { ...defaults, serversEnabled: false })).toBe(true);
  });

  it('rebases only fields changed by this browser after a CAS conflict', () => {
    const remote: DashboardPreferences = {
      ...defaults,
      navigationOrder: ['home', 'overview', 'storage', 'network', 'host', 'terminal', 'servers', 'hooks'],
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
});
