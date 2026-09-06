import { describe, expect, it } from 'vitest';
import { render } from 'preact-render-to-string';
import { ServerRuntimeControls, runtimeSoftware } from './server-runtime';
import type { NativeServerDetail } from './control-api';

const detail = { id: 'helix:test', name: 'Survival', kind: 'minecraft', software: 'Paper', minecraftVersion: '1.21.1', build: '123', modpack: null } as NativeServerDetail;
const props = { detail, csrfToken: 'test', canManage: true, onComplete: () => {}, onSessionExpired: () => {}, onBackups: () => {} };
describe('runtime controls', () => {
  it('shows repair, version choice and recovery with explicit confirmation', () => {
    const html = render(<ServerRuntimeControls {...props} />);
    expect(html).toContain('Repair current runtime');
    expect(html).toContain('Choose version / update build');
    expect(html).toContain('Stopped servers stay stopped');
    expect(html).toContain('Back up Survival');
    expect(html).toMatch(/disabled[^>]*>Back up &amp; repair/);
  });
  it('keeps pack loaders pinned but offers exact repair', () => {
    const html = render(<ServerRuntimeControls {...props} detail={{ ...detail, modpack: {} as NonNullable<NativeServerDetail['modpack']> }} />);
    expect(html).toContain('Repair current runtime');
    expect(html).not.toContain('Choose version / update build');
  });
  it('does not offer unsupported custom or non-Minecraft migrations', () => {
    expect(runtimeSoftware('Custom JAR')).toBeNull();
    expect(render(<ServerRuntimeControls {...props} detail={{ ...detail, kind: 'valheim' }} />)).toBe('');
    expect(runtimeSoftware('NeoForge')).toBe('neoforge');
  });
});
