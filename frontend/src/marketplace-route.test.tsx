import { describe, expect, it } from 'vitest';
import { loadMarketplaceRoute } from './marketplace-route';

describe('Marketplace route loader', () => {
  it('shares one in-flight import and resolves the real panel', async () => {
    const first = loadMarketplaceRoute();
    const second = loadMarketplaceRoute();

    expect(second).toBe(first);
    await expect(first).resolves.toBeTypeOf('function');
  });
});
