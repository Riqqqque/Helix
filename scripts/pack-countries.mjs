import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

function parseCsv(text) {
  const rows = [];
  let row = [];
  let cell = '';
  let quoted = false;
  for (let index = 0; index < text.length; index += 1) {
    const character = text[index];
    if (quoted) {
      if (character === '"' && text[index + 1] === '"') {
        cell += '"';
        index += 1;
      } else if (character === '"') {
        quoted = false;
      } else {
        cell += character;
      }
    } else if (character === '"') {
      quoted = true;
    } else if (character === ',') {
      row.push(cell);
      cell = '';
    } else if (character === '\n' || character === '\r') {
      if (character === '\r' && text[index + 1] === '\n') index += 1;
      row.push(cell);
      if (row.some((value) => value.length > 0)) rows.push(row);
      row = [];
      cell = '';
    } else {
      cell += character;
    }
  }
  if (cell.length > 0 || row.length > 0) {
    row.push(cell);
    rows.push(row);
  }
  return rows;
}

const extras = {
  XK: { name: 'Kosovo', lat: 42.6, lon: 20.9 },
  PS: { name: 'Palestine', lat: 31.95, lon: 35.23 },
  EH: { name: 'Western Sahara', lat: 24.2, lon: -12.9 },
  BQ: { name: 'Caribbean Netherlands', lat: 12.2, lon: -68.3 },
  SX: { name: 'Sint Maarten', lat: 18.04, lon: -63.05 },
  CW: { name: 'Curacao', lat: 12.17, lon: -68.99 },
  SS: { name: 'South Sudan', lat: 6.88, lon: 31.31 },
  HK: { name: 'Hong Kong', lat: 22.32, lon: 114.17 },
  SG: { name: 'Singapore', lat: 1.35, lon: 103.82 },
  MO: { name: 'Macao', lat: 22.2, lon: 113.54 },
  TW: { name: 'Taiwan', lat: 23.7, lon: 121 },
  VA: { name: 'Vatican City', lat: 41.9, lon: 12.45 },
  GI: { name: 'Gibraltar', lat: 36.14, lon: -5.35 },
  SM: { name: 'San Marino', lat: 43.94, lon: 12.45 },
  MC: { name: 'Monaco', lat: 43.73, lon: 7.42 },
  LI: { name: 'Liechtenstein', lat: 47.17, lon: 9.51 },
  AD: { name: 'Andorra', lat: 42.51, lon: 1.52 },
  MT: { name: 'Malta', lat: 35.9, lon: 14.5 },
  IM: { name: 'Isle of Man', lat: 54.23, lon: -4.55 },
  JE: { name: 'Jersey', lat: 49.21, lon: -2.13 },
  GG: { name: 'Guernsey', lat: 49.45, lon: -2.58 },
  AX: { name: 'Aland Islands', lat: 60.2, lon: 20 },
  BV: { name: 'Bouvet Island', lat: -54.42, lon: 3.35 },
  AQ: { name: 'Antarctica', lat: -80, lon: 0 },
};

const csvRows = parseCsv(await readFile(path.join(process.env.TEMP, 'country-centroids.csv'), 'utf8'));
const header = csvRows.shift() ?? [];
const lonIndex = header.indexOf('longitude');
const latIndex = header.indexOf('latitude');
const isoIndex = header.indexOf('ISO');
const nameIndex = header.indexOf('COUNTRY');
const byIso = new Map();
for (const row of csvRows) {
  const iso = (row[isoIndex] ?? '').trim();
  const lat = Number(row[latIndex]);
  const lon = Number(row[lonIndex]);
  if (iso.length === 2 && Number.isFinite(lat) && Number.isFinite(lon)) {
    byIso.set(iso, { name: row[nameIndex], lat, lon });
  }
}

const bin = await readFile(new URL('../crates/helix-privd/data/ipv4-country.bin', import.meta.url));
const count = bin.readUInt16LE(6);
let offset = 16;
const codes = [];
for (let index = 0; index < count; index += 1) {
  const end = bin.indexOf(10, offset);
  codes.push(bin.subarray(offset, end).toString('ascii'));
  offset = end + 1;
}

const names = new Intl.DisplayNames(['en'], { type: 'region' });
const countries = [];
const missing = [];
for (const code of codes) {
  const extra = extras[code] ?? byIso.get(code);
  const name = extra?.name ?? names.of(code) ?? code;
  if (!Number.isFinite(extra?.lat) || !Number.isFinite(extra?.lon)) {
    missing.push(code);
    continue;
  }
  countries.push({
    code,
    name,
    lat: Math.round(extra.lat * 100) / 100,
    lon: Math.round(extra.lon * 100) / 100,
  });
}
if (missing.length > 0) {
  throw new Error(`missing centroids: ${missing.join(',')}`);
}

await writeFile(new URL('../crates/helix-privd/data/countries.json', import.meta.url), JSON.stringify(countries));
console.log(`wrote ${countries.length} countries`);
