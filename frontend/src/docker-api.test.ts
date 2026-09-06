import { describe, expect, it } from 'vitest';
import { parseDockerInventory, parseHomarrCatalog } from './docker-api';

describe('Homarr catalog API', () => {
  const ready = {
    schema_version: 1,
    availability: 'ready',
    container: 'homarr',
    widgets: [
      { name: 'Plex', url: 'http://192.168.1.10:32400/web', icon: 'https://example.test/plex.png', x: 0, y: 0, width: 2 },
      { name: 'Radarr', url: 'http://192.168.1.10:7878', icon: null },
    ],
    note: 'Helix places these on a Homarr Home in Homarr layout order and matches icons from names, links, or Homarr icon slugs. Uploaded Homarr files stay in Homarr.',
    collected_at_unix_ms: 1_800_000_000_000,
  };

  it('parses Homarr shortcuts and keeps http(s) icons', () => {
    expect(parseHomarrCatalog(ready)).toMatchObject({
      availability: 'ready',
      container: 'homarr',
      widgets: [
        { name: 'Plex', url: 'http://192.168.1.10:32400/web', icon: 'https://example.test/plex.png', x: 0, y: 0, width: 2 },
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

describe('Docker inventory API', () => {
  const inventory = {
    schema_version: 1,
    availability: 'ready',
    docker_installed: true,
    truncated: false,
    error: null,
    note: 'Helix lists Docker Engine containers on this host.',
    collected_at_unix_ms: 1_800_000_000_000,
    portainer: {
      detected: true,
      container: 'portainer',
      running: true,
      panel_port: 9443,
      panel_scheme: 'https',
    },
    containers: [
      {
        name: 'AMP_AllTheMons01',
        image: 'cubecoders/ampbase',
        state: 'running',
        status: 'Up 3 days',
        ports: '',
        running: true,
        protected: false,
        cpu_percent: 0,
        memory_used_bytes: 12_288,
        memory_limit_bytes: 10_737_418_240,
        pids: 12,
        panel_port: null,
      },
      {
        name: 'homarr',
        image: 'ghcr.io/homarr-labs/homarr:latest',
        state: 'exited',
        status: 'Exited (0)',
        ports: '',
        running: false,
        protected: false,
        cpu_percent: null,
        memory_used_bytes: null,
        memory_limit_bytes: null,
        pids: null,
        panel_port: null,
      },
    ],
  };

  it('keeps containers that have empty ports or images', () => {
    const parsed = parseDockerInventory(inventory);
    expect(parsed.dockerInstalled).toBe(true);
    expect(parsed.containers).toHaveLength(2);
    expect(parsed.containers[0]).toMatchObject({
      name: 'AMP_AllTheMons01',
      ports: '',
      running: true,
      memoryUsedBytes: 12_288,
    });
    expect(parsed.containers[1]).toMatchObject({ name: 'homarr', image: 'ghcr.io/homarr-labs/homarr:latest', running: false });
  });

  it('skips one bad container instead of hiding the whole list', () => {
    const parsed = parseDockerInventory({
      ...inventory,
      containers: [
        inventory.containers[0],
        { ...inventory.containers[1], name: '../escape' },
      ],
    });
    expect(parsed.containers.map((item) => item.name)).toEqual(['AMP_AllTheMons01']);
  });
});
