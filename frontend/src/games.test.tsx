import renderToString from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { visualGameInstances, visualGameReadiness } from './game-fixtures';
import { filterGameInstances, GameWorkspaceView } from './games';

describe('Games workspace', () => {
  it('shows the real locked boundary without sample servers or enabled controls', () => {
    const html = renderToString(
      <GameWorkspaceView
        readiness={{ ...visualGameReadiness, availability: 'unavailable' }}
        readinessPhase="ready"
        instances={null}
      />,
    );

    expect(html).toContain('Your servers will appear here.');
    expect(html).toContain('Game hosting is safely locked');
    expect(html).not.toContain('Survival SMP');
    expect(html.match(/disabled/g)?.length).toBeGreaterThanOrEqual(2);
  });

  it('renders a bounded server overview with status, players, resources, and safety', () => {
    const html = renderToString(
      <GameWorkspaceView
        readiness={visualGameReadiness}
        readinessPhase="ready"
        instances={visualGameInstances}
        totalInstances={visualGameInstances.length}
      />,
    );

    expect(html).toContain('Survival SMP');
    expect(html).toContain('Night Shift');
    expect(html).toContain('18 / 40');
    expect(html).toContain('6 GiB / 8 GiB');
    expect(html).toContain('2 warnings');
    expect(html).toContain('4 of 4 servers shown');
  });

  it('filters by normalized identity and operational attention state', () => {
    expect(filterGameInstances(visualGameInstances, 'night', 'all').map(({ name }) => name))
      .toEqual(['Night Shift']);
    expect(filterGameInstances(visualGameInstances, '', 'online').map(({ name }) => name))
      .toEqual(['Survival SMP']);
    expect(filterGameInstances(visualGameInstances, '', 'attention').map(({ name }) => name))
      .toEqual(['Night Shift']);
  });

  it('keeps high-cardinality filtering bounded to the supplied page', () => {
    const page = Array.from({ length: 100 }, (_, index) => ({
      ...visualGameInstances[0]!,
      id: `${String(index).padStart(8, '0')}-b7af-4f13-9f56-0559788b2c56`,
      name: `Server ${String(index).padStart(3, '0')}`,
      status: index % 2 === 0 ? 'online' as const : 'offline' as const,
    }));

    expect(filterGameInstances(page, 'server 09', 'online')).toHaveLength(5);
    expect(filterGameInstances(page, '', 'offline')).toHaveLength(50);
  });
});
