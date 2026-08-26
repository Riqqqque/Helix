import { afterEach, describe, expect, it, vi } from 'vitest';
import render from 'preact-render-to-string';
import { Dashboard, HostPanel, NetworkPanel, StoragePanel } from './app';

const unavailableOverview = {
  data: null,
  phase: 'error' as const,
  error: 'Review probe failed.',
};

const readyOverview = {
  data: {
    hostname: 'helix-node',
    operatingSystem: 'Linux',
    architecture: 'x86_64',
    kernelVersion: '6.17.0',
    uptimeSeconds: 90,
    cpu: { usagePercent: 12.5, logicalCores: 8 },
    memory: {
      totalBytes: 8_589_934_592,
      usedBytes: 2_147_483_648,
      availableBytes: 6_442_450_944,
    },
    swap: { totalBytes: 0, usedBytes: 0 },
    storage: {
      availability: 'available' as const,
      mounts: [
        {
          name: 'nvme0n1p2',
          fileSystem: 'ext4',
          mountPoint: '/',
          totalBytes: 1_099_511_627_776n,
          availableBytes: 824_633_720_832n,
          usedBytes: 274_877_906_944n,
          readOnly: false,
          removable: false,
        },
      ],
      omittedMounts: 0,
      omittedTextFields: 0,
    },
    network: {
      availability: 'available' as const,
      interfaces: [
        {
          name: 'enp1s0',
          addresses: [{ address: '192.0.2.10', prefixLength: 24 }],
          totalReceivedBytes: 8_589_934_592n,
          totalTransmittedBytes: 2_147_483_648n,
          mtuBytes: 1_500n,
        },
      ],
      omittedInterfaces: 0,
      omittedAddresses: 0,
    },
    collectedAtUnixMs: 1_788_000_000_000,
  },
  phase: 'ready' as const,
  error: null,
};

const dashboardProps = {
  user: {
    id: '019c7714-3b77-44d1-9866-e1f484aae2ab',
    loginName: 'rique.owner',
    displayName: 'Rique',
    capabilities: ['system.view'],
  },
  csrfToken: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
  onSessionExpired: () => undefined,
  onLogout: () => Promise.resolve(),
};

function renderDashboard(): string {
  return render(<Dashboard {...dashboardProps} />);
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('dashboard panel failure states', () => {
  it('keeps host, storage, and network failure copy attached to the right panel', () => {
    expect(render(<HostPanel overview={unavailableOverview} />)).toContain(
      'Host details unavailable',
    );
    expect(render(<StoragePanel overview={unavailableOverview} />)).toContain(
      'Storage details unavailable',
    );
    expect(render(<NetworkPanel overview={unavailableOverview} />)).toContain(
      'Network details unavailable',
    );
  });
});

describe('dashboard navigation accessibility', () => {
  it('marks the current fragment in both responsive navigation surfaces', () => {
    const markup = renderDashboard();

    expect(markup.match(/aria-current="location"/gu)).toHaveLength(2);
  });

  it('keeps the topbar context synchronized with the URL fragment', () => {
    vi.stubGlobal('window', { location: { hash: '#network' } });

    const markup = renderDashboard();
    const context = markup.match(/<div class="topbar__context">.*?<\/div>/u)?.[0];

    expect(context).toContain('<span>Network</span>');
    expect(context).not.toContain('<span>Overview</span>');
  });
});

describe('dashboard accessibility semantics', () => {
  it('exposes one atomic live status and a focusable entry heading', () => {
    const markup = renderDashboard();

    expect(markup).toMatch(
      /<div(?=[^>]*\bclass="hero__status-card")(?=[^>]*\brole="status")(?=[^>]*\baria-live="polite")(?=[^>]*\baria-atomic="true")[^>]*>/u,
    );
    expect(markup).toMatch(
      /<h1(?=[^>]*\bid="overview-title")(?=[^>]*\btabindex="-1")[^>]*>/u,
    );
    expect(markup).toContain('>Source &amp; license</a>');
  });

  it('natively disables refresh while the initial request is in flight', () => {
    const markup = renderDashboard();
    const refreshButton = markup.match(/<button(?=[^>]*\bclass="refresh-button")[^>]*>/u)?.[0];

    expect(refreshButton).toContain('disabled');
    expect(refreshButton).toContain('aria-busy="true"');
  });

  it('does not turn metric and discovery cards into repeated landmarks', () => {
    const dashboardMarkup = renderDashboard();
    const discoveryMarkup = render(
      <>
        <StoragePanel overview={readyOverview} />
        <NetworkPanel overview={readyOverview} />
      </>,
    );

    expect(dashboardMarkup.match(/class="metric-card(?: |")/gu)).toHaveLength(4);
    expect(dashboardMarkup).not.toContain('<article');
    expect(discoveryMarkup.match(/class="discovery-card"/gu)).toHaveLength(2);
    expect(discoveryMarkup).not.toContain('<article');
  });

  it('renders every bounded host-discovery entry at the API ceiling', () => {
    const baseMount = readyOverview.data.storage.mounts[0]!;
    const baseInterface = readyOverview.data.network.interfaces[0]!;
    const boundaryOverview = {
      ...readyOverview,
      data: {
        ...readyOverview.data,
        storage: {
          ...readyOverview.data.storage,
          mounts: Array.from({ length: 64 }, (_, index) => ({
            ...baseMount,
            name: `disk-${index}`,
            mountPoint: `/srv/game-${index}`,
          })),
        },
        network: {
          ...readyOverview.data.network,
          interfaces: Array.from({ length: 64 }, (_, interfaceIndex) => ({
            ...baseInterface,
            name: `game-net-${interfaceIndex}`,
            addresses: Array.from({ length: 4 }, (_, addressIndex) => ({
              address: `192.0.${interfaceIndex}.${addressIndex + 1}`,
              prefixLength: 24,
            })),
          })),
        },
      },
    };

    const markup = render(
      <>
        <StoragePanel overview={boundaryOverview} />
        <NetworkPanel overview={boundaryOverview} />
      </>,
    );

    expect(markup.match(/class="discovery-card"/gu)).toHaveLength(128);
    expect(markup.match(/192\.0\.\d+\.\d+\/24/gu)).toHaveLength(256);
  });
});
