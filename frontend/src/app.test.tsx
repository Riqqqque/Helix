import { describe, expect, it } from 'vitest';
import render from 'preact-render-to-string';
import { Dashboard, HostPanel, NetworkPanel, StoragePanel } from './app';

const unavailableOverview = {
  data: null,
  phase: 'error' as const,
  error: 'Review probe failed.',
};

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
    const markup = render(
      <Dashboard
        user={{
          id: '019c7714-3b77-44d1-9866-e1f484aae2ab',
          loginName: 'rique.owner',
          displayName: 'Rique',
          capabilities: ['system.view'],
        }}
        csrfToken="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        onSessionExpired={() => undefined}
        onLogout={() => Promise.resolve()}
      />,
    );

    expect(markup.match(/aria-current="location"/gu)).toHaveLength(2);
  });
});
