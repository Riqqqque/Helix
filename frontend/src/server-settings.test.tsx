import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { SettingsPanel } from './servers';
import type { MinecraftSettings, NativeServerDetail } from './control-api';

const settings: MinecraftSettings = {
  expectedRevision: 'a'.repeat(64), motd: 'Survival', gameMode: 'survival', difficulty: 'normal',
  maxPlayers: 20, viewDistance: 10, simulationDistance: 8, playerIdleTimeout: 0,
  onlineMode: true, pvp: true, allowFlight: true, whiteList: false, enforceWhiteList: false,
  spawnProtection: 16, gamePort: 25565, memoryMb: 4096,
  restartBehavior: { activation: 'server_restart', restartRequiredFields: ['allow_flight'], message: 'Restart to apply' },
};
const detail = { id: 'helix:test', name: 'Survival', kind: 'minecraft', software: 'Paper', settings, status: 'online' } as NativeServerDetail & { settings: MinecraftSettings };
const props = { detail, csrfToken: 'test', servers: [], canManageServers: true, canManageNetwork: false, restartSuccessRevision: 0, onRestart: () => {}, onSaved: () => {}, onSessionExpired: () => {} };
describe('Minecraft settings controls', () => {
  it('shows the saved toggle and separates save from restart', () => {
    const html = render(<SettingsPanel {...props} />);
    expect(html).toContain('Save &amp; restart');
    expect(html).toContain('This does not grant flying');
    expect(html).toMatch(/type="checkbox" checked[^>]*>[\s\S]*?Allow flight/);
    expect(html).toContain('Settings match the saved file');
  });
  it('keeps unsupported Pumpkin flight controls disabled', () => {
    const html = render(<SettingsPanel {...props} detail={{ ...detail, software: 'Pumpkin', settings: { ...settings, allowFlight: false, spawnProtection: 0 } }} />);
    expect(html).toMatch(/type="checkbox" disabled[^>]*>[\s\S]*?Allow flight/);
  });
  it('does not let read-only users save settings', () => {
    const html = render(<SettingsPanel {...props} canManageServers={false} />);
    expect(html).toMatch(/disabled[^>]*>Save settings/);
    expect(html).toMatch(/disabled[^>]*>Save &amp; restart/);
  });
});
