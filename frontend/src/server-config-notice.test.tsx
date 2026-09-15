import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { ServerConfigNotice } from './server-config-notice';
import type { NativeServerDetail } from './control-api';

const detail = { status: 'online' } as NativeServerDetail;
describe('Saved configuration notice', () => {
  it('shows one quiet notice for saved Minecraft settings without another restart button', () => {
    const html = render(<ServerConfigNotice detail={{ ...detail, configChanges: { state: 'changed', files: ['server.properties'], limited: true } }} />);
    expect(html).toContain('Settings saved.');
    expect(html).toContain('Your server is still running.');
    expect(html).toContain('server.properties');
    expect(html).toContain('View changed files');
    expect(html).not.toContain('<button');
    expect(html).not.toContain('<details open');
    expect(html).not.toContain('1+');
    expect(html).not.toContain('Some mods reload');
    expect(html).toContain('bounded check');
  });
  it('keeps mixed-file and incomplete-scan explanations inside the disclosure', () => {
    const html = render(<ServerConfigNotice detail={{ ...detail, configChanges: { state: 'changed', files: ['server.properties', 'plugins/example/config.yml'], limited: true } }} />);
    expect(html.indexOf('does not confirm')).toBeGreaterThan(html.indexOf('<details>'));
    expect(html.indexOf('bounded check')).toBeGreaterThan(html.indexOf('<details>'));
  });
  it('does not show a restart notice for a stopped server or missing evidence', () => {
    expect(render(<ServerConfigNotice detail={detail} />)).toBe('');
    expect(render(<ServerConfigNotice detail={{ ...detail, status: 'stopped', configChanges: { state: 'changed', files: ['server.properties'], limited: false } }} />)).toBe('');
  });
});
