import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { ServerTokenSettings, catalog, tokenStatus } from './server-token-settings';

describe('server token settings', () => {
  it('loads token management on demand without exposing any credentials', () => {
    const html = render(<ServerTokenSettings csrfToken="never-display-this-proof" servers={[]} />);
    expect(html).toContain('Server API tokens');
    expect(html).toContain('Manage tokens');
    expect(html).not.toContain('never-display-this-proof');
    expect(html).not.toContain('New API token');
  });

  it('reads persistent token metadata without mistaking it for an expired token', () => {
    const data = catalog({ permissions: ['view'], tokens: [{
      id: 'one', name: 'Automation', servers: ['helix:one'], permissions: ['view'],
      expires_at: null, revoked_at: null,
    }] });
    expect(data.tokens[0]?.expiresAt).toBeNull();
    expect(data.tokens[0]?.revoked).toBe(false);
    expect(tokenStatus(data.tokens[0]!, Date.now()).label).toBe('Active · never expires');
  });

  it('reports tokens that can no longer authenticate', () => {
    const base = { id: 'one', name: 'Automation', servers: ['helix:one'], permissions: ['view'], created_at: 1, expires_at: 5_000, last_used_at: 2, revoked_at: null };
    const [live, expired, invalid, revoked] = catalog({ permissions: ['view'], tokens: [
      { ...base, authorized: true }, { ...base, expires_at: 10 }, { ...base, authorized: false }, { ...base, revoked_at: 3 },
    ] }).tokens;
    expect(tokenStatus(live!, 100).usable).toBe(true);
    expect(tokenStatus(expired!, 100).label).toBe('Expired');
    expect(tokenStatus(invalid!, 100).usable).toBe(false);
    expect(tokenStatus(revoked!, 100).label).toBe('Revoked');
  });
});
