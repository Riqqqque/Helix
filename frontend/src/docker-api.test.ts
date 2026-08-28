import { describe, expect, it } from 'vitest';
import { parseHomarrCatalog } from './docker-api';

describe('Homarr catalog API', () => {
  const ready = {
    schema_version: 1,
    availability: 'ready',
    container: 'homarr',
    widgets: [
      { name: 'Plex', url: 'http://192.168.1.10:32400/web', icon: 'https://example.test/plex.png' },
      { name: 'Radarr', url: 'http://192.168.1.10:7878', icon: null },
    ],
    note: 'Choose which Homarr links to place on this Home.',
    collected_at_unix_ms: 1_800_000_000_000,
  };

  it('parses Homarr shortcuts and keeps http(s) icons', () => {
    expect(parseHomarrCatalog(ready)).toMatchObject({
      availability: 'ready',
      container: 'homarr',
      widgets: [
        { name: 'Plex', url: 'http://192.168.1.10:32400/web', icon: 'https://example.test/plex.png' },
        { name: 'Radarr', url: 'http://192.168.1.10:7878', icon: null },
      ],
    });
  });

  it('accepts an honest empty catalog when Homarr has no importable apps', () => {
    expect(parseHomarrCatalog({
      ...ready,
      widgets: [],
      note: "Homarr's app catalog has no http(s) addresses Helix can import.",
    }).widgets).toEqual([]);
  });

  it('rejects javascript shortcuts', () => {
    expect(() => parseHomarrCatalog({
      ...ready,
      widgets: [{ name: 'Evil', url: 'javascript:alert(1)', icon: null }],
    })).toThrow(/invalid shortcut/i);
  });
});
