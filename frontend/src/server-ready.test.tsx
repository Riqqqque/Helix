import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { ServerReadySummary } from './server-ready';

const props = { name: 'Sky server', host: '192.168.1.5', port: 25567, elapsed: '1m 41s', pack: 'Sky 2.0.2', runtime: 'Minecraft 1.21.1 · NeoForge', hostRequested: true, firewallState: 'ufw_inactive_not_blocking', hostError: null };

describe('server ready summary', () => {
  it('shows both Pumpkin client protocols without forwarding the dashboard', () => {
    const html = render(<ServerReadySummary {...props} pumpkin bedrockPort={25568} />);
    expect(html).toContain('TCP (Java)');
    expect(html).toContain('TCP + UDP 25568');
    expect(html).toContain('not your Helix dashboard');
  });
  it('separates a successful install from manual router forwarding', () => {
    const html = render(<ServerReadySummary {...props} />);
    expect(html).toContain('192.168.1.5:25567');
    expect(html).toContain('25567 → 25567');
    expect(html).toContain('TCP');
    expect(html).toContain('no UFW rule needed');
    expect(html).toContain('Router settings were not changed');
    expect(html).not.toContain('needs attention');
    expect(html).not.toContain('100%');
  });
  it('shows host problems once without saying server creation failed', () => {
    const html = render(<ServerReadySummary {...props} hostError="Status unavailable" />);
    expect(html.split('Status unavailable')).toHaveLength(2);
    expect(html).toContain('Your server is running');
  });
  it('does not guess an unknown LAN address', () => {
    expect(render(<ServerReadySummary {...props} host="" />)).toContain('Find this host’s LAN address');
  });
});
