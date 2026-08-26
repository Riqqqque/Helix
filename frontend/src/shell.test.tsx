import { describe, expect, it } from 'vitest';
import render from 'preact-render-to-string';
import { App } from './shell';

describe('authentication shell accessibility', () => {
  it('moves skip-link navigation to a programmatically focusable main region', () => {
    const markup = render(<App />);

    expect(markup).toContain('class="skip-link" href="#main-content"');
    expect(markup).toMatch(
      /<main(?=[^>]*\bid="main-content")(?=[^>]*\btabindex="-1")[^>]*>/u,
    );
    expect(markup).toContain(
      'href="https://github.com/Riqqqque/Helix" target="_blank" rel="noopener noreferrer"',
    );
  });
});
