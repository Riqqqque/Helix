import { afterEach, describe, expect, it, vi } from 'vitest';
import render from 'preact-render-to-string';
import { Dashboard } from './app';

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
  it('keeps Servers with the primary pages and pins Settings below them', () => {
    const markup = render(<Dashboard {...dashboardProps} />);
    const sidebar = markup.match(/<nav[^>]*class="sidebar-nav"[^>]*>.*?<\/nav>/u)?.[0] ?? '';
    const links = Array.from(sidebar.matchAll(/href="([^"]+)"/gu), (match) => match[1]);

    expect(links).toEqual(['#overview', '#home', '#storage', '#network', '#host', '#terminal', '#servers', '#hooks', '#settings']);
    expect(sidebar).toContain('>Arrange<');
    expect(sidebar).toMatch(/nav-item nav-item--settings/u);
    expect(markup).not.toContain('Control plane');
    expect(markup).not.toContain('Your server clearly in view');
  });

  it('renders every navigation destination with a page heading or route loading boundary', () => {
    for (const [hash, title] of [
      ['#overview', 'Overview'],
      ['#home', 'Home'],
      ['#storage', 'Storage'],
      ['#network', 'Network'],
      ['#host', 'Host'],
      ['#terminal', 'Terminal'],
      ['#servers', 'Servers'],
      ['#hooks', 'Hooks'],
      ['#settings', 'Settings'],
    ] as const) {
      vi.stubGlobal('window', { location: { hash } });
      const markup = render(<Dashboard {...dashboardProps} />);
      if (hash === '#home' || hash === '#hooks' || hash === '#terminal') {
        expect(markup).toContain(hash === '#home' ? 'Loading Home…' : hash === '#hooks' ? 'Loading Hooks…' : 'Loading terminal…');
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
