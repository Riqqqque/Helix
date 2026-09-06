import {
  ApiError,
  expectArray,
  expectNumber,
  expectRecord,
  expectString,
  requestJson,
} from './api';

export type GlobeLinkKind = 'player' | 'outbound';

export interface GlobeOrigin {
  available: boolean;
  country: string | null;
  countryName: string | null;
  lat: number | null;
  lon: number | null;
  precision: 'country' | 'unknown';
  label: string;
  note: string;
}

export interface GlobeLink {
  id: string;
  kind: GlobeLinkKind;
  country: string;
  countryName: string;
  lat: number;
  lon: number;
  peers: number;
  activity: number;
  servers: string[];
}

export interface GlobeSnapshot {
  schemaVersion: number;
  collectedAtUnixMs: number;
  origin: GlobeOrigin;
  links: GlobeLink[];
  truncated: boolean;
  note: string;
}

function expectBoolean(record: Record<string, unknown>, key: string, context: string): boolean {
  const value = record[key];
  if (typeof value !== 'boolean') {
    throw new ApiError(`${context} returned an invalid ${key} value.`);
  }
  return value;
}

function optionalText(value: unknown): string | null {
  return typeof value === 'string' && value.trim().length > 0 ? value : null;
}

function optionalCoordinate(value: unknown, minimum: number, maximum: number): number | null {
  if (value === null || value === undefined) return null;
  if (typeof value !== 'number' || !Number.isFinite(value) || value < minimum || value > maximum) {
    throw new ApiError('Globe snapshot returned an invalid coordinate.');
  }
  return value;
}

export function parseGlobeSnapshot(value: unknown): GlobeSnapshot {
  const context = 'Globe snapshot';
  const root = expectRecord(value, context);
  const originRecord = expectRecord(root.origin, `${context} origin`);
  const available = expectBoolean(originRecord, 'available', `${context} origin`);
  const precision = originRecord.precision === 'country' || originRecord.precision === 'unknown'
    ? originRecord.precision
    : 'unknown';
  const origin: GlobeOrigin = {
    available,
    country: optionalText(originRecord.country),
    countryName: optionalText(originRecord.country_name),
    lat: optionalCoordinate(originRecord.lat, -90, 90),
    lon: optionalCoordinate(originRecord.lon, -180, 180),
    precision,
    label: expectString(originRecord, 'label', `${context} origin`),
    note: expectString(originRecord, 'note', `${context} origin`),
  };
  if (origin.available && (origin.lat === null || origin.lon === null || origin.country === null)) {
    throw new ApiError('Globe snapshot origin was marked available without a country pin.');
  }
  const links = expectArray(root, 'links', context, 48).map((item, index) => {
    const record = expectRecord(item, `${context} link ${index}`);
    let kind: GlobeLinkKind;
    if (record.kind === 'player') kind = 'player';
    else if (record.kind === 'outbound') kind = 'outbound';
    else throw new ApiError(`${context} returned an invalid link kind.`);
    const lat = optionalCoordinate(record.lat, -90, 90);
    const lon = optionalCoordinate(record.lon, -180, 180);
    if (lat === null || lon === null) {
      throw new ApiError(`${context} returned a link without coordinates.`);
    }
    const servers = expectArray(record, 'servers', `${context} link ${index}`, 24).map((server) => {
      if (typeof server !== 'string' || server.trim().length === 0) {
        throw new ApiError(`${context} returned an invalid server name.`);
      }
      return server;
    });
    return {
      id: expectString(record, 'id', `${context} link ${index}`),
      kind,
      country: expectString(record, 'country', `${context} link ${index}`),
      countryName: expectString(record, 'country_name', `${context} link ${index}`),
      lat,
      lon,
      peers: expectNumber(record, 'peers', `${context} link ${index}`, { integer: true, minimum: 1, maximum: 8_192 }),
      activity: expectNumber(record, 'activity', `${context} link ${index}`, { minimum: 0, maximum: 1 }),
      servers,
    };
  });
  return {
    schemaVersion: expectNumber(root, 'schema_version', context, { integer: true, minimum: 1, maximum: 1 }),
    collectedAtUnixMs: expectNumber(root, 'collected_at_unix_ms', context, { integer: true, minimum: 0 }),
    origin,
    links,
    truncated: expectBoolean(root, 'truncated', context),
    note: expectString(root, 'note', context),
  };
}

export function getGlobeSnapshot(csrfToken: string, signal?: AbortSignal): Promise<GlobeSnapshot> {
  return requestJson('/api/v1/network/globe', parseGlobeSnapshot, {
    csrfToken,
    signal,
    timeoutMs: 12_000,
  });
}
