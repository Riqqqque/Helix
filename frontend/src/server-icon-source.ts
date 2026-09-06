import type { ManagedServer } from './control-api';

export function parseModpackIconUrl(value: unknown): string | null {
  if (typeof value !== 'string' || value.length > 2_048) return null;
  try {
    const url = new URL(value, 'http://helix.invalid');
    const match = /^\/api\/v1\/marketplace\/(modrinth|curseforge)\/image\?/u.exec(value);
    if (match === null || url.origin !== 'http://helix.invalid' || url.hash !== '') return null;
    const keys = Array.from(url.searchParams.keys());
    const path = url.searchParams.get('path');
    const prefix = match[1] === 'modrinth' ? '/data/' : '/avatars/';
    if (keys.length !== 1 || keys[0] !== 'path' || path === null || path.length > 512
      || !path.startsWith(prefix) || !/^\/[A-Za-z0-9._/-]+$/u.test(path)
      || path.split('/').some((segment) => segment === '.' || segment === '..')
      || !/\.(?:png|jpe?g|webp|gif)$/iu.test(path)) return null;
    return value;
  } catch {
    return null;
  }
}

export function serverIconSource(server: Pick<ManagedServer, 'appearance' | 'modpackIconUrl'>): string | null {
  if (server.appearance.kind === 'custom') return server.appearance.imageUrl;
  if (server.appearance.kind === 'preset') return null;
  return parseModpackIconUrl(server.modpackIconUrl);
}
