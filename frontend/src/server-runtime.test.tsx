import { describe, expect, it } from 'vitest';
import { render } from 'preact-render-to-string';
import { ServerRuntimeControls, runtimeModes, runtimeResultNote, runtimeSoftware } from './server-runtime';
import type { NativeServerDetail } from './control-api';

const detail = { id: 'helix:test', name: 'Survival', kind: 'minecraft', software: 'Paper', minecraftVersion: '1.21.1', build: '123', javaVersion: 21, status: 'online', modpack: null } as unknown as NativeServerDetail;
const props = { detail, csrfToken: 'test', canManage: true, onComplete: () => {}, onSessionExpired: () => {}, onBackups: () => {} };
describe('runtime controls', () => {
  it('offers update, version change and repair with one explicit confirmation', () => {
    const html = render(<ServerRuntimeControls {...props} />);
    expect(html).toContain('Update build');
    expect(html).toContain('Change version');
    expect(html).toContain('Repair files');
    expect(html).toContain('Helix makes a full backup first.');
    expect(html).toContain('restores the backup automatically');
    expect(html).toContain('Back up Survival and update build');
    expect(html).toMatch(/disabled[^>]*>Back up &amp; update/);
  });
  it('tells stopped servers they stay stopped', () => {
    const html = render(<ServerRuntimeControls {...props} detail={{ ...detail, status: 'stopped' } as NativeServerDetail} />);
    expect(html).toContain('The server stays stopped.');
  });
  it('keeps pack loaders pinned but offers exact repair', () => {
    const html = render(<ServerRuntimeControls {...props} detail={{ ...detail, modpack: { projectTitle: 'All the Mods' } } as unknown as NativeServerDetail} />);
    expect(html).toContain('Repair files');
    expect(html).not.toContain('Change version');
    expect(html).not.toContain('Update build');
    expect(html).toContain('All the Mods');
  });
  it('treats Pumpkin as releases, not builds', () => {
    expect(runtimeModes('pumpkin', false).map((mode) => mode.id)).toEqual(['version', 'repair']);
    expect(runtimeModes('pumpkin', false)[0]?.title).toBe('Change release');
  });
  it('explains what a finished job did', () => {
    expect(runtimeResultNote({ already_current: true })).toContain('Already on the newest build');
    expect(runtimeResultNote({ version: '1.21.1', build: '130', runtime_validation_performed: true })).toBe('Now on 1.21.1 build 130. The server started cleanly.');
    expect(runtimeResultNote({ version: '1.21.2', build: '5', runtime_validation_performed: false })).toContain('Start the server when you are ready.');
  });
  it('does not offer unsupported custom or non-Minecraft migrations', () => {
    expect(runtimeSoftware('Custom JAR')).toBeNull();
    expect(render(<ServerRuntimeControls {...props} detail={{ ...detail, kind: 'valheim' } as NativeServerDetail} />)).toBe('');
    expect(runtimeSoftware('NeoForge')).toBe('neoforge');
  });
});
