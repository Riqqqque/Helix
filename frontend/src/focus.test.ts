import { describe, expect, it, vi } from 'vitest';
import { focusOnMount } from './focus';

describe('view entry focus', () => {
  it('focuses a mounted target and tolerates ref cleanup', () => {
    const focus = vi.fn();
    const target = { focus } as unknown as HTMLElement;

    focusOnMount(target);
    focusOnMount(null);

    expect(focus).toHaveBeenCalledOnce();
  });
});
