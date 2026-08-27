import { describe, expect, it } from 'vitest';
import { loadSettingsRoute } from './settings-route';

describe('Settings route loader', () => {
  it('shares one in-flight import and resolves the real route', async () => {
    const first = loadSettingsRoute();
    const second = loadSettingsRoute();
    expect(second).toBe(first);
    await expect(first).resolves.toBeTypeOf('function');
  });
});
