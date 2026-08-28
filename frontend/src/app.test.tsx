import { afterEach, describe, expect, it, vi } from 'vitest';
import render from 'preact-render-to-string';
import { Dashboard, DiskMap } from './app';
import type { HostInventory } from './control-api';

const dashboardProps = {
  user: {
    id: '019c7714-3b77-44d1-9866-e1f484aae2ab',
    loginName: 'rique.owner',
    displayName: 'Rique',
    capabilities: ['system.view'],
  },
  csrfToken: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
  onSessionExpired: () => undefined,
  onAccountUpdated: () => undefined,
  onLogout: () => Promise.resolve(),
};

afterEach(() => vi.unstubAllGlobals());

describe('dashboard shell', () => {
  it('puts an explicit space analyzer action on every mounted physical drive', () => {
    const inventory: HostInventory = {
      disks: [{ name: 'sda', path: '/dev/sda', parent: null, deviceType: 'disk', sizeBytes: 1_000_000_000_000, fileSystem: null, label: null, mountPoints: ['/'], model: 'WD Boot', serial: null, transport: 'sata', rotational: false, readOnly: false, hotplug: false }],
      mounts: [{ target: '/', source: '/dev/sda2', fileSystem: 'ext4', sizeBytes: 900_000_000_000, usedBytes: 810_000_000_000, availableBytes: 90_000_000_000, usePercent: 90, readOnly: false }],
      interfaces: [], routes: [], listeners: [], services: [], processes: [], loadAverage: [0, 0, 0], processCount: 128, cpuModel: 'AMD Ryzen', collectedAtUnixMs: 1,
    };
    const markup = render(<DiskMap inventory={inventory} onBrowse={() => undefined} onAnalyze={() => undefined} />);

    expect(markup).toContain('WD Boot');
    expect(markup).toContain('Browse files');
    expect(markup).toContain('Analyze space');
  });

  it('keeps Servers with the primary pages and pins Settings below them', () => {
    const markup = render(<Dashboard {...dashboardProps} />);
    const sidebar = markup.match(/<nav[^>]*class="sidebar-nav"[^>]*>.*?<\/nav>/u)?.[0] ?? '';
    const links = Array.from(sidebar.matchAll(/href="([^"]+)"/gu), (match) => match[1]);

    expect(links).toEqual(['#overview', '#home', '#storage', '#network', '#host', '#security', '#terminal', '#servers', '#hooks', '#strands', '#settings']);
    expect(sidebar).toContain('>Arrange<');
    expect(sidebar).toMatch(/nav-item nav-item--settings/u);
    expect(markup).not.toContain('Control plane');
    expect(markup).not.toContain('Your server clearly in view');
  });

  it('hides Servers until the owner enables the module', () => {
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => (key === 'helix.dashboard.servers-enabled' ? 'false' : null),
      setItem: () => undefined,
      removeItem: () => undefined,
    });
    const sidebarMarkup = render(<Dashboard {...dashboardProps} />);
    const sidebar = sidebarMarkup.match(/<nav[^>]*class="sidebar-nav"[^>]*>.*?<\/nav>/u)?.[0] ?? '';
    expect(sidebar).not.toContain('#servers');

    vi.stubGlobal('window', { location: { hash: '#servers' } });
    const page = render(<Dashboard {...dashboardProps} />);
    expect(page).toContain('Game servers are turned off');
    expect(page).toContain('Enable Servers');
    expect(page).not.toContain('Loading server controls…');
  });

  it('renders every navigation destination with a page heading or route loading boundary', () => {
    for (const [hash, title] of [
      ['#overview', 'Overview'],
      ['#home', 'Home'],
      ['#storage', 'Storage'],
      ['#network', 'Network'],
      ['#host', 'Host'],
      ['#security', 'Security'],
      ['#terminal', 'Terminal'],
      ['#servers', 'Servers'],
      ['#hooks', 'Hooks'],
      ['#strands', 'Strands'],
      ['#settings', 'Settings'],
    ] as const) {
      vi.stubGlobal('window', { location: { hash } });
      const markup = render(<Dashboard {...dashboardProps} />);
      if (hash === '#home' || hash === '#hooks' || hash === '#terminal' || hash === '#overview' || hash === '#strands') {
        expect(markup).toContain(hash === '#home' ? 'Loading Home…' : hash === '#hooks' ? 'Loading Hooks…' : hash === '#overview' ? 'Loading Overview…' : hash === '#strands' ? 'Loading Strands…' : 'Loading terminal…');
      } else {
        expect(markup).toContain(`<h1>${title}</h1>`);
      }
    }
  });

  it('shows the owner name without manufacturing a profile avatar', () => {
    const markup = render(<Dashboard {...dashboardProps} />);
    expect(markup).toContain('<span class="account-name">Rique</span>');
    expect(markup).not.toContain('account-avatar');
  });

  it('renders an accessible lightweight fallback while Settings load', () => {
    vi.stubGlobal('window', { location: { hash: '#settings' } });
    const markup = render(<Dashboard {...dashboardProps} />);
    expect(markup).toContain('aria-busy="true"');
    expect(markup).toContain('role="status"');
    expect(markup).toContain('Loading settings…');
  });

  it('renders an accessible lightweight fallback while Server controls load', () => {
    vi.stubGlobal('window', { location: { hash: '#servers' } });
    const markup = render(<Dashboard {...dashboardProps} />);

    expect(markup).toContain('aria-busy="true"');
    expect(markup).toContain('role="status"');
    expect(markup).toContain('Loading server controls…');
    expect(markup).toContain('Helix’s own game-server manager');
    expect(markup).not.toContain('New servers use Helix’s native manager—not AMP');
  });

  it('renders an accessible lightweight fallback while Home loads', () => {
    vi.stubGlobal('window', { location: { hash: '#home' } });
    const markup = render(<Dashboard {...dashboardProps} />);

    expect(markup).toContain('aria-busy="true"');
    expect(markup).toContain('Loading Home…');
  });

  it('keeps the main region keyboard reachable and marks one current page', () => {
    const markup = render(<Dashboard {...dashboardProps} />);

    expect(markup).toMatch(/<main(?=[^>]*id="main-content")(?=[^>]*tabindex="-1")[^>]*>/u);
    expect(markup.match(/aria-current="page"/gu)).toHaveLength(2);
  });
});
