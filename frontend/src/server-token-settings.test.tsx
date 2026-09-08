import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { ServerTokenSettings } from './server-token-settings';

describe('server token settings', () => {
  it('loads token management on demand without exposing any credentials', () => {
    const html = render(<ServerTokenSettings csrfToken="never-display-this-proof" servers={[]} />);
    expect(html).toContain('Server API tokens');
    expect(html).toContain('Manage tokens');
    expect(html).not.toContain('never-display-this-proof');
    expect(html).not.toContain('New API token');
  });
});
