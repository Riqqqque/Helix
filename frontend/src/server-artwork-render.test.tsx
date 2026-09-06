import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import type { ManagedServer } from './control-api';
import { ServerArtwork } from './server-artwork';

const server = {
  status: 'online', kind: 'minecraft', software: 'NeoForge',
  appearance: { kind: 'default', revision: 0 },
  modpackIconUrl: '/api/v1/marketplace/curseforge/image?path=%2Favatars%2F12%2F345%2Ficon.png',
} as ManagedServer;

describe('modpack server artwork rendering', () => {
  it.each(['row', 'detail'] as const)('renders the inherited image in the %s view', (size) => {
    const markup = render(<ServerArtwork server={server} size={size} />);
    expect(markup).toContain(`<img src="${server.modpackIconUrl}"`);
    expect(markup).toContain(`server-artwork--${size}`);
    expect(markup).toContain('loading="lazy"');
  });

  it('renders the existing game mark when no artwork was saved', () => {
    const markup = render(<ServerArtwork server={{ ...server, modpackIconUrl: null }} />);
    expect(markup).not.toContain('<img');
  });
});
