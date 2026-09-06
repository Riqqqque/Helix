import { afterEach, describe, expect, it, vi } from 'vitest';
import { isCurseforgeKeyRequired, getCurseforgeKeyStatus, normalizeCurseforgeApiKey } from './curseforge-key-api';

describe('CurseForge catalog key', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('recognizes the Settings Catalogs setup message', () => {
    expect(isCurseforgeKeyRequired('CurseForge needs an API key. Open Settings → Catalogs, paste a key from console.curseforge.com, then search again.')).toBe(true);
    expect(isCurseforgeKeyRequired('CurseForge catalog was unreachable.')).toBe(false);
    expect(isCurseforgeKeyRequired("CurseForge's CDN blocked this host's public IP. That is not a bad key. Search needs this host to reach the internet without a VPS or VPN exit they block.")).toBe(false);
  });

  it('unwraps quotes, docker dollar escaping, and wrapped lines from a console key', () => {
    const key = '$2a$10$abcdefghijklmnopqrstuvwx';
    expect(normalizeCurseforgeApiKey(`  '${key}'  `)).toBe(key);
    expect(normalizeCurseforgeApiKey(`"${key}"`)).toBe(key);
    expect(normalizeCurseforgeApiKey('$$2a$$10$$abcdefghijklmnopqrstuvwx')).toBe(key);
    expect(normalizeCurseforgeApiKey(`${key.slice(0, 16)}\n${key.slice(16)}`)).toBe(key);
    expect(normalizeCurseforgeApiKey('$2a$10$abc/defGHIJK.lmnopqrstuv')).toBe('$2a$10$abc/defGHIJK.lmnopqrstuv');
    expect(normalizeCurseforgeApiKey(`CF_API_KEY=$$2a$$10$$${key.slice('$2a$10$'.length)}\u200b`)).toBe(key);
    expect(normalizeCurseforgeApiKey(`Copy this: ${key} please\u00a0`)).toBe(key);
    expect(normalizeCurseforgeApiKey('\uFF042a\uFF0410\uFF04abcdefghijklmnopqrstuvwx')).toBe(key);
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

  it('parses a CDN-blocked probe after save', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({
      schema_version: 1,
      configured: true,
      catalog: 'api.curseforge.com',
      probe: 'cdn_blocked',
    }), { status: 200, headers: { 'Content-Type': 'application/json' } })));
    await expect(getCurseforgeKeyStatus('csrf')).resolves.toEqual({
      configured: true,
      catalog: 'api.curseforge.com',
      probe: 'cdn_blocked',
    });
  });
});
