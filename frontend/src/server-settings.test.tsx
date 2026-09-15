import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { ServerConfigNotice, SettingsPanel, minecraftPvpUsesGameRule } from './servers';
import { parseServerConfigChanges } from './control-api';
import type { MinecraftSettings, NativeServerDetail } from './control-api';

const settings: MinecraftSettings = {
  expectedRevision: 'a'.repeat(64), motd: 'Survival', gameMode: 'survival', difficulty: 'normal',
  maxPlayers: 20, viewDistance: 10, simulationDistance: 8, playerIdleTimeout: 0,
  onlineMode: true, pvp: true, allowFlight: true, whiteList: false, enforceWhiteList: false,
  spawnProtection: 16, gamePort: 25565, memoryMb: 4096,
  restartBehavior: { activation: 'server_restart', restartRequiredFields: ['allow_flight'], message: 'Restart to apply' },
};
const detail = { id: 'helix:test', name: 'Survival', kind: 'minecraft', software: 'Paper', minecraftVersion: '1.21.1', settings, status: 'online' } as NativeServerDetail & { settings: MinecraftSettings };
const props = { detail, csrfToken: 'test', servers: [], canManageServers: true, canManageNetwork: false, restartSuccessRevision: 0, onRestart: () => {}, onSaved: () => {}, onSessionExpired: () => {} };
describe('Minecraft settings controls', () => {
  it('restores a configuration warning from server data after opening a fresh page', () => {
    const changed = { ...detail, configChanges: { state: 'changed' as const, files: ['world/serverconfig/ftbchunks-world.snbt'], limited: false } };
    const html = render(<ServerConfigNotice detail={changed} />);
    expect(html).toContain('Configuration files changed.');
    expect(html).toContain('world/serverconfig/ftbchunks-world.snbt');
    expect(html).toContain('A file change alone does not confirm that a mod loaded it');
    expect(html).toContain('View changed files');
    expect(html).not.toContain('<button');
  });
  it('does not advertise applied settings when no changes or runtime evidence are available', () => {
    for (const state of ['unknown', 'no_changes', 'stopped'] as const) {
      expect(render(<ServerConfigNotice detail={{ ...detail, configChanges: { state, files: [], limited: false } }} />)).toBe('');
    }
  });
  it('validates the bounded config-change response and supports older servers', () => {
    expect(parseServerConfigChanges(undefined)).toBeNull();
    expect(parseServerConfigChanges({ state: 'changed', files: ['config/test.toml'], limited: true })).toEqual({ state: 'changed', files: ['config/test.toml'], limited: true });
    expect(() => parseServerConfigChanges({ state: 'applied', files: [], limited: false })).toThrow();
    expect(() => parseServerConfigChanges({ state: 'changed', files: Array(33).fill('config/test.toml'), limited: false })).toThrow();
  });
  it('uses game-rule guidance for modern PvP instead of an ineffective toggle', () => {
    expect(minecraftPvpUsesGameRule('Paper', '1.21.8')).toBe(false);
    for (const version of ['1.21.9', '1.21.11', '26.2']) {
      expect(minecraftPvpUsesGameRule('Paper', version)).toBe(true);
    }
    expect(minecraftPvpUsesGameRule('Pumpkin', '26.2')).toBe(false);
    expect(render(<SettingsPanel {...props} detail={{ ...detail, minecraftVersion: '26.2' }} />)).toContain('gamerule pvp false');
  });
  it('shows the saved toggle and separates save from restart', () => {
    const html = render(<SettingsPanel {...props} />);
    expect(html).not.toContain('Save &amp; restart');
    expect(html).not.toContain('restart-field');
    expect(html).not.toContain('Restart now');
    expect(html).toContain('This does not grant flying');
    expect(html).toMatch(/type="checkbox" checked[^>]*>[\s\S]*?Allow flight/);
    expect(html).toContain('Settings match the saved file');
  });
  it('keeps unsupported Pumpkin flight controls disabled', () => {
    const html = render(<SettingsPanel {...props} detail={{ ...detail, software: 'Pumpkin', settings: { ...settings, allowFlight: false, spawnProtection: 0 } }} />);
    expect(html).toMatch(/type="checkbox" disabled[^>]*>[\s\S]*?Allow flight/);
  });
  it('retains the restart status after reopening saved settings without adding another action', () => {
    const html = render(<SettingsPanel {...props} detail={{ ...detail, configChanges: { state: 'changed', files: ['server.properties'], limited: true } }} />);
    expect(html).toContain('Saved · restart when ready');
    expect(html).not.toContain('Restart now');
    expect(html).not.toContain('settings-restart-choice');
    expect(html).not.toContain('Save &amp; restart');
  });
  it('does not let read-only users save settings', () => {
    const html = render(<SettingsPanel {...props} canManageServers={false} />);
    expect(html).toMatch(/disabled[^>]*>Save settings/);
    expect(html).not.toContain('Save &amp; restart');
  });
});
