import { describe, expect, it } from 'vitest';
import { parseGlobeSnapshot } from './globe-api';

const sample = {
  schema_version: 1,
  collected_at_unix_ms: 1_800_000_000_000,
  origin: {
    available: true,
    country: 'US',
    country_name: 'United States',
    lat: 38.82,
    lon: -96.33,
    precision: 'country',
    label: 'This host',
    note: 'Placed from the router public WAN address.',
  },
  links: [
    {
      id: 'NL:player',
      kind: 'player',
      country: 'NL',
      country_name: 'Netherlands',
      lat: 52.13,
      lon: 5.55,
      peers: 4,
      activity: 0.72,
      servers: ['Survival'],
    },
    {
      id: 'DE:outbound',
      kind: 'outbound',
      country: 'DE',
      country_name: 'Germany',
      lat: 51.08,
      lon: 10.43,
      peers: 2,
      activity: 0.2,
      servers: [],
    },
  ],
  truncated: false,
  note: 'Pins are country-level. Helix never sends remote addresses to the browser.',
};

describe('globe snapshot parser', () => {
  it('maps broker snake_case into the dashboard model', () => {
    const parsed = parseGlobeSnapshot(sample);
    expect(parsed.origin.countryName).toBe('United States');
    expect(parsed.links[0]).toMatchObject({
      kind: 'player',
      country: 'NL',
      peers: 4,
      servers: ['Survival'],
    });
    expect(parsed.links[1]?.kind).toBe('outbound');
  });

  it('allows an unplaced host without inventing coordinates', () => {
    const parsed = parseGlobeSnapshot({
      ...sample,
      origin: {
        available: false,
        country: null,
        country_name: null,
        lat: null,
        lon: null,
        precision: 'unknown',
        label: 'This host',
        note: 'No public WAN address.',
      },
    });
    expect(parsed.origin.available).toBe(false);
    expect(parsed.origin.lat).toBeNull();
  });

  it('rejects remote-looking payload that includes extra link kinds', () => {
    expect(() => parseGlobeSnapshot({
      ...sample,
      links: [{ ...sample.links[0], kind: 'probe' }],
    })).toThrow(/link kind/u);
  });
});
