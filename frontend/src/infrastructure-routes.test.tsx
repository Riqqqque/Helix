import { describe, expect, it } from 'vitest';
import { loadHostUpdatesRoute, loadNetworkOperationsRoute } from './infrastructure-routes';

describe('infrastructure route loaders', () => {
  it('shares the network chunk import', async () => {
    const first = loadNetworkOperationsRoute();
    expect(loadNetworkOperationsRoute()).toBe(first);
    await expect(first).resolves.toBeTypeOf('function');
  });

  it('shares the host updates chunk import', async () => {
    const first = loadHostUpdatesRoute();
    expect(loadHostUpdatesRoute()).toBe(first);
    await expect(first).resolves.toBeTypeOf('function');
  });
});
