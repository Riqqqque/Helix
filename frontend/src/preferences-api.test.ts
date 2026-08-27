import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  getDashboardPreferences,
  parseDashboardPreferencesRecord,
  putDashboardPreferences,
  type DashboardPreferencesRecord,
} from './preferences-api';

const validRecord: DashboardPreferencesRecord = {
  revision: 4,
  preferences: {
    navigationOrder: ['overview', 'home', 'storage', 'network', 'host', 'terminal', 'servers', 'hooks'],
    metricsRefreshMs: 1_000,
    homeWidgets: [{ id: 'clock', kind: 'clock', size: 'compact', height: 'medium', title: 'Right now', content: '', url: '', color: '' }],
    homeTemplates: [{ id: 'home-main', name: 'Main', accent: '#d7f64d', widgets: [{ id: 'clock', kind: 'clock', size: 'compact', height: 'medium', title: 'Right now', content: '', url: '', color: '' }] }],
    activeHomeId: 'home-main',
    colors: { accent: '', text: '', surface: '' },
  },
  updatedAtUnixMs: 1_800_000_000_000,
};

afterEach(() => vi.unstubAllGlobals());

describe('dashboard preferences API', () => {
  it('parses a bounded preference record', () => {
    expect(parseDashboardPreferencesRecord(validRecord)).toEqual(validRecord);
  });

  it('accepts the unset revision contract with no update timestamp', () => {
    expect(parseDashboardPreferencesRecord({
      ...validRecord,
      revision: 0,
      updatedAtUnixMs: null,
    })).toMatchObject({ revision: 0, updatedAtUnixMs: null });
  });

  it('rejects missing navigation pages and unsupported refresh rates', () => {
    expect(() => parseDashboardPreferencesRecord({
      ...validRecord,
      preferences: { ...validRecord.preferences, navigationOrder: ['overview'] },
    })).toThrow();
    expect(() => parseDashboardPreferencesRecord({
      ...validRecord,
      preferences: { ...validRecord.preferences, metricsRefreshMs: 3_000 },
    })).toThrow();
  });

  it('loads with CSRF and writes with an expected revision', async () => {
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(new Response(JSON.stringify(validRecord), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    })));
    vi.stubGlobal('fetch', fetchMock);
    const csrf = 'EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE';

    await getDashboardPreferences(csrf);
    await putDashboardPreferences(4, validRecord.preferences, csrf);

    const [getPath, getRequest] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(getPath).toBe('/api/v1/settings/preferences');
    expect(getRequest.headers).toMatchObject({ 'X-Helix-CSRF': csrf });
    const [putPath, putRequest] = fetchMock.mock.calls[1] as [string, RequestInit];
    expect(putPath).toBe('/api/v1/settings/preferences');
    expect(putRequest.method).toBe('PUT');
    expect(JSON.parse(String(putRequest.body))).toMatchObject({ expectedRevision: 4 });
  });
});
