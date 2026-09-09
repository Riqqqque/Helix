import { describe, expect, it } from 'vitest';
import { render } from 'preact-render-to-string';
import { defaultValheimSettings, parseValheimMod, parseValheimSettings, parseValheimStatus } from './valheim-api';
import { ValheimSettingsFields } from './valheim-panel';

describe('Valheim configuration', () => {
  it('round-trips all launch settings, including empty defaults', () => {
    const settings = defaultValheimSettings();
    expect(parseValheimSettings(settings)).toEqual(settings);
    settings.modifiers = { combat: 'hard', resources: 'more' };
    settings.keys = ['nomap']; settings.crossplay = true;
    expect(parseValheimSettings(settings)).toEqual(settings);
  });
  it('rejects malformed settings rather than silently resetting controls', () => {
    expect(() => parseValheimSettings({ ...defaultValheimSettings(), crossplay: 'false' })).toThrow();
    expect(() => parseValheimSettings({ ...defaultValheimSettings(), keys: null })).toThrow();
    expect(() => parseValheimStatus({ settings: defaultValheimSettings(), mods: [] })).toThrow();
  });
  it('constructs package links from validated identities', () => {
    const value = { package: 'Author-Package', version: '1.0.0', description: 'A mod', dependencies: [], enabled: true, deprecated: false, url: 'javascript:alert(1)' };
    expect(parseValheimMod(value).url).toBe('https://thunderstore.io/c/valheim/p/Author/Package/');
    expect(() => parseValheimMod({ ...value, package: '../outside' })).toThrow();
  });
  it('explains world preservation, crossplay and preset reset behavior', () => {
    const html = render(<ValheimSettingsFields value={defaultValheimSettings()} onChange={() => {}} disabled={false} creating />);
    expect(html).toContain('does not rename or delete');
    expect(html).toContain('not a LAN or loopback IP');
    expect(html).toContain('resets its modifiers');
    expect(html).toContain('Leave blank to generate');
    expect(html).toContain('automatic backups');
    expect(html).not.toContain('value="64"');
  });
  it('disables all controls during a save or while the server is running', () => {
    const html = render(<ValheimSettingsFields value={defaultValheimSettings()} onChange={() => {}} disabled />);
    expect(html.match(/<(?:input|select)\b/g)?.length).toBe(html.match(/ disabled/g)?.length);
  });
});
