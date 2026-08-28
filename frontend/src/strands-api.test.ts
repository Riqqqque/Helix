import { describe, expect, it } from 'vitest';
import { parseStrandInventory } from './strands-api';

const inventory = {
  strands: [
    {
      id: 'b893d568-327d-4b6e-b0b6-0b7a58e0c852',
      slug: 'system-health',
      name: 'System Health',
      version: '0.1.0',
      description: 'Shows a bounded summary of host health.',
      license: 'AGPL-3.0-or-later',
      publisher: 'Helix example',
      kind: 'ui-only',
      enabled: true,
      origin: 'upload',
      originDetail: 'system-health.strand.zip',
      digestSha256: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      uiEntry: 'ui/index.html',
      capabilities: [
        { name: 'helix:metrics.read', reason: 'Read the selected node health summary.', optional: false, origins: [] },
      ],
      packageBytes: 4096,
      installedAtUnixMs: 1_800_000_000_000,
      updatedAtUnixMs: 1_800_000_000_100,
      hasPage: true,
      hasWidget: true,
    },
  ],
};

describe('strands API', () => {
  it('accepts a bounded Strand inventory', () => {
    expect(parseStrandInventory(inventory)[0]).toMatchObject({
      slug: 'system-health',
      enabled: true,
      hasWidget: true,
    });
  });

  it('rejects a malformed Strand id', () => {
    expect(() => parseStrandInventory({
      strands: [{ ...inventory.strands[0], id: 'not-a-uuid' }],
    })).toThrow();
  });
});
