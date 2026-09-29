import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import type { ManagedServer, NativeServerDetail } from './control-api';
import { ServerTransferCard, defaultTransferParts, parseTransferPreflight, transferParts, transferResultNote, transferTargets } from './server-transfer';

const server = (id: string, name: string, extra: Partial<ManagedServer> = {}) => ({ id, name, kind: 'minecraft', manager: 'helix', software: 'Paper', version: '26.2', ...extra }) as unknown as ManagedServer;
const detail = { id: 'helix:test', name: 'TestServer', kind: 'minecraft', software: 'Paper', minecraftVersion: '26.2', status: 'online' } as unknown as NativeServerDetail;
const props = { detail, csrfToken: 'x', canManage: true, onComplete: () => {}, onSessionExpired: () => {} };

describe('server transfer', () => {
  it('offers only Helix Minecraft servers other than the source', () => {
    const targets = transferTargets([
      server('helix:test', 'TestServer'),
      server('helix:prod', 'Production'),
      server('amp:doinks', 'DoinksServer', { manager: 'amp_import' }),
      server('helix:valheim', 'Valheim', { kind: 'valheim' } as Partial<ManagedServer>),
    ], 'helix:test');
    expect(targets.map((entry) => entry.id)).toEqual(['helix:prod']);
  });

  it('matches parts to the software and keeps risky parts opt-in', () => {
    expect(defaultTransferParts('Paper')).toEqual(['plugins', 'configs', 'datapacks']);
    expect(defaultTransferParts('NeoForge')).toEqual(['mods', 'configs', 'datapacks']);
    expect(transferParts('Paper').map((part) => part.id)).toContain('plugin_data');
    expect(transferParts('Fabric').map((part) => part.id)).not.toContain('plugins');
    expect(transferParts('Paper').filter((part) => part.caution).map((part) => part.id)).toEqual(['player_lists', 'plugin_data', 'worlds']);
  });

  it('explains how to get a target when only AMP servers exist', () => {
    const html = render(<ServerTransferCard {...props} servers={[server('helix:test', 'TestServer'), server('amp:doinks', 'DoinksServer', { manager: 'amp_import' })]} />);
    expect(html).toContain('import them into Helix first');
    expect(render(<ServerTransferCard {...props} servers={[server('helix:prod', 'Production')]} />)).toContain('Check what will be copied');
  });

  it('reads the check and summarises the finished copy', () => {
    const check = parseTransferPreflight({ parts: { plugins: { files: 3, bytes: 2048, items: ['AllFather-0.9.18.jar', 'AllFather'] } }, total_bytes: 2048, errors: [], warnings: ['versions differ'], target: { running: true } });
    expect(check.parts.plugins?.files).toBe(3);
    expect(check.targetRunning).toBe(true);
    expect(transferResultNote({ files_copied: 3, replaced_jars: ['plugins/AllFather-0.9.17.jar'], removed_jars: [], restarted: true, backup_id: '1' }))
      .toBe('Copied 3 files. Replaced 1 older JAR. The server started again cleanly. Its previous state is in Backups.');
  });
});
