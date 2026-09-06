import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { CopyButton, copyFlashLabel } from './copy-button';

describe('copy button', () => {
  it('starts as Copy and names the control for the clipboard action', () => {
    const markup = render(<CopyButton text="192.168.1.10:25565" />);
    expect(markup).toContain('Copy');
    expect(markup).toContain('copy-button');
    expect(markup).toContain('aria-live="polite"');
    expect(markup).not.toContain('Copied');
  });

  it('labels copied and failed flashes in plain language', () => {
    expect(copyFlashLabel('idle')).toBe('Copy');
    expect(copyFlashLabel('idle', 'Copy all')).toBe('Copy all');
    expect(copyFlashLabel('copied')).toBe('Copied');
    expect(copyFlashLabel('failed')).toBe('Couldn’t copy');
  });
});
