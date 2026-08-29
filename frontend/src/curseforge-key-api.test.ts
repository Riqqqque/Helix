import { afterEach, describe, expect, it, vi } from 'vitest';
import { isCurseforgeKeyRequired, getCurseforgeKeyStatus } from './curseforge-key-api';

describe('CurseForge catalog key', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('recognizes the Settings Catalogs setup message', () => {
    expect(isCurseforgeKeyRequired('CurseForge needs an API key. Open Settings → Catalogs, paste a key from console.curseforge.com, then search again.')).toBe(true);
    expect(isCurseforgeKeyRequired('CurseForge catalog was unreachable.')).toBe(false);
  });

  it('parses a configured status without returning the key', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({
      schema_version: 1,
      configured: true,
      catalog: 'api.curseforge.com',
    }), { status: 200, headers: { 'Content-Type': 'application/json' } })));
    await expect(getCurseforgeKeyStatus('csrf')).resolves.toEqual({
      configured: true,
      catalog: 'api.curseforge.com',
    });
  });
});
