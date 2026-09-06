import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { defaultMarketplaceVersion, marketplaceInstallRuntimeCopy, MarketplacePanel } from './marketplace';
import type { MarketplaceVersion } from './marketplace-api';

function version(id: string, versionType: MarketplaceVersion['versionType'], hasPrimaryFile = true): MarketplaceVersion {
  return {
    id,
    name: id,
    versionNumber: id,
    versionType,
    datePublished: null,
    downloads: 0,
    gameVersions: ['1.21.4'],
    loaders: ['paper'],
    hasPrimaryFile,
  };
}

describe('Marketplace panel', () => {
  it('defaults to the latest returned release with a usable primary file', () => {
    const versions = [version('new-beta', 'beta'), version('broken-release', 'release', false), version('latest-release', 'release'), version('old-release', 'release')];
    expect(defaultMarketplaceVersion(versions)?.id).toBe('latest-release');
  });

  it('renders a native plugin marketplace without loading remote project images', () => {
    const markup = render(<MarketplacePanel
      server={{ id: 'native_survival', name: 'Survival', software: 'Paper', minecraftVersion: '1.21.4', status: 'online' }}
      csrfToken="csrf"
      canManageServers
      onSessionExpired={() => undefined}
      onInstalled={async () => undefined}
    />);

    expect(markup).toContain('Plugin marketplace');
    expect(markup).toContain('Modrinth');
    expect(markup).toContain('CurseForge');
    expect(markup).toContain('Search plugins by name');
    expect(markup).toContain('Settings → Catalogs');
    expect(markup).toContain('If you use CurseForge');
    expect(markup).toContain('normal ISP IP');
    expect(markup).not.toContain('without an API key');
    expect(markup).not.toContain('<img');
  });

  it('does not promise runtime rollback for a stopped server', () => {
    const stopped = marketplaceInstallRuntimeCopy('stopped');
    const running = marketplaceInstallRuntimeCopy('online');

    expect(stopped.backup).toContain('leaves the server stopped');
    expect(stopped.validation).toContain('Restart the server yourself');
    expect(running.backup).toContain('leaves the server running');
    expect(running.validation).toContain('Restart the server yourself');
  });
});
