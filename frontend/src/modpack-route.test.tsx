import { describe, expect, it } from 'vitest';
import { loadModpackPicker } from './modpack-route';

describe('Modpack picker route', () => {
  it('shares one nested lazy import and resolves the real picker', async () => {
    const first = loadModpackPicker();
    const second = loadModpackPicker();

    expect(second).toBe(first);
    await expect(first).resolves.toBeTypeOf('function');
  });
});
