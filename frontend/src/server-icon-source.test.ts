import { describe, expect, it } from 'vitest';
import { parseModpackIconUrl, serverIconSource } from './server-icon-source';

const modrinth = '/api/v1/marketplace/modrinth/image?path=%2Fdata%2Fpack%2Ficon.png';
const curseforge = '/api/v1/marketplace/curseforge/image?path=%2Favatars%2F12%2F345%2Ficon.webp';

describe('inherited modpack artwork', () => {
  it.each([modrinth, curseforge])('uses safe pack artwork as the default: %s', (url) => {
    expect(serverIconSource({ appearance: { kind: 'default', revision: 0 }, modpackIconUrl: url })).toBe(url);
  });

  it.each([undefined, null, '', 'https://example.com/icon.png', '//example.com/icon.png',
    '/api/v1/marketplace/modrinth/image?path=%2Fdata%2F..%2Fsecret.png',
    `${modrinth}&extra=true`, `${modrinth}#fragment`,
    '/api/v1/marketplace/curseforge/image?path=%2Favatars%2Ficon.svg'])('falls back for absent or unsafe artwork: %s', (url) => {
    expect(parseModpackIconUrl(url)).toBeNull();
  });

  it('keeps a chosen preset instead of the pack artwork', () => {
    expect(serverIconSource({ appearance: { kind: 'preset', preset: 'grass', revision: 1, updatedAtUnixMs: 1 }, modpackIconUrl: modrinth })).toBeNull();
  });

  it('keeps an uploaded image instead of the pack artwork', () => {
    expect(serverIconSource({ appearance: { kind: 'custom', revision: 1, updatedAtUnixMs: 1,
      contentType: 'image/png', width: 512, height: 512, imageUrl: '/custom-icon' }, modpackIconUrl: modrinth })).toBe('/custom-icon');
  });

  it('uses the pack again after restoring the default', () => {
    expect(serverIconSource({ appearance: { kind: 'default', revision: 0 }, modpackIconUrl: curseforge })).toBe(curseforge);
  });
});
