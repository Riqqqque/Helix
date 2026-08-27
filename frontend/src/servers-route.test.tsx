import { describe, expect, it } from 'vitest';
import { loadServersRoute } from './servers-route';

describe('Servers route loader', () => {
  it('shares one in-flight import and resolves the real route', async () => {
    const first = loadServersRoute();
    const second = loadServersRoute();

    expect(second).toBe(first);
    await expect(first).resolves.toBeTypeOf('function');
  });
});
