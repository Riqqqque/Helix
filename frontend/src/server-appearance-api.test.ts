import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  clearServerIcon,
  parseServerAppearance,
  setServerCustomIcon,
  setServerIconPreset,
} from './server-appearance-api';

afterEach(() => vi.unstubAllGlobals());

describe('server appearance API', () => {
  it('accepts only same-origin revisioned image paths', () => {
    expect(parseServerAppearance({ kind: 'default', revision: 0 })).toEqual({ kind: 'default', revision: 0 });
    expect(parseServerAppearance({
      kind: 'custom',
      revision: 3,
      content_type: 'image/png',
      width: 512,
      height: 256,
      updated_at_unix_ms: 1_800_000_000_000,
      image_url: '/api/v1/servers/helix:server/appearance/image?revision=3',
    })).toMatchObject({ kind: 'custom', revision: 3, width: 512 });
    expect(() => parseServerAppearance({
      kind: 'custom', revision: 3, content_type: 'image/png', width: 64, height: 64,
      updated_at_unix_ms: 1, image_url: 'https://tracking.example/icon.png?revision=3',
    })).toThrow(/unsafe image URL/u);
  });

  it('sends revision-guarded preset, custom, and clear mutations', async () => {
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(
      new Response(JSON.stringify({ kind: 'preset', revision: 2, preset: 'portal', updated_at_unix_ms: 2 }), { status: 200 }),
    ));
    vi.stubGlobal('fetch', fetchMock);
    await setServerIconPreset('helix:server', 'portal', 1, 'csrf');
    await setServerCustomIcon('helix:server', 'image/jpeg', 'YWJj', 1, 'csrf');
    await clearServerIcon('helix:server', 1, 'csrf');
    const requests = fetchMock.mock.calls.map(([, options]) => JSON.parse(String((options as RequestInit).body)) as Record<string, unknown>);
    expect(requests).toEqual([
      { kind: 'preset', expected_revision: 1, preset: 'portal' },
      { kind: 'custom', expected_revision: 1, content_type: 'image/jpeg', image_base64: 'YWJj' },
      { expected_revision: 1 },
    ]);
    expect((fetchMock.mock.calls[2]?.[1] as RequestInit).method).toBe('DELETE');
  });
});
