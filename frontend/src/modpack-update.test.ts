import { describe, expect, it } from 'vitest';
import type { NativeInstalledModpack } from './control-api';
import type { ModpackVersion } from './modpack-api';
import { selectCompatibleModpackUpdate } from './servers';

const installed: NativeInstalledModpack = {
  provider: 'curseforge',
  projectId: '925200',
  projectTitle: 'All the Mods 10',
  versionId: 'current',
  versionName: 'ATM10 8.1',
  versionNumber: '8.1',
  minecraftVersion: '1.21.1',
  loader: 'neoforge',
  loaderVersion: '21.1.249',
};

function version(
  id: string,
  published: string,
  overrides: Partial<ModpackVersion> = {},
): ModpackVersion {
  return {
    id,
    name: id,
    versionNumber: id,
    versionType: 'release',
    status: 'available',
    datePublished: published,
    downloads: 0,
    gameVersions: ['1.21.1'],
    loaders: ['neoforge'],
    installable: true,
    compatibilityReason: 'Compatible',
    mrpackFile: {
      filename: `${id}.zip`,
      size: 1024,
      modrinthDeclaredSha512Available: false,
    },
    ...overrides,
  };
}

describe('modpack update selection', () => {
  it('chooses the newest compatible release by date, not response order', () => {
    const selected = selectCompatibleModpackUpdate(installed, [
      version('older', '2026-08-01T00:00:00Z'),
      version('newest', '2026-09-03T00:00:00Z'),
      version('current', '2026-08-20T00:00:00Z'),
      version('newer', '2026-08-30T00:00:00Z'),
    ]);
    expect(selected?.id).toBe('newest');
  });

  it('does not offer cross-version, cross-loader, beta, or older files', () => {
    expect(
      selectCompatibleModpackUpdate(installed, [
        version('fabric', '2026-09-03T00:00:00Z', { loaders: ['fabric'] }),
        version('minecraft', '2026-09-04T00:00:00Z', { gameVersions: ['1.22'] }),
        version('beta', '2026-09-05T00:00:00Z', { installable: false }),
        version('current', '2026-08-20T00:00:00Z'),
        version('older', '2026-08-01T00:00:00Z'),
      ]),
    ).toBeNull();
  });

  it('fails closed when the installed release is absent from a truncated page', () => {
    expect(() =>
      selectCompatibleModpackUpdate(installed, [
        version('newest', '2026-09-03T00:00:00Z'),
      ]),
    ).toThrow(/does not include this installed release/i);
  });
});
