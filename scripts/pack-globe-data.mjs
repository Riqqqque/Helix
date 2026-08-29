import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Packs the CC0 NRO whois IPv4 country CSV into crates/helix-privd/data/ipv4-country.bin
// and Natural Earth 110m land TopoJSON into frontend/src/globe-land.ts.
// Download geo-ipv4-num.csv and land-110m.json into %TEMP% /tmp first.

const root = fileURLToPath(new URL('..', import.meta.url));

function decodeTopojsonLand(topology) {
  const { transform, arcs } = topology;
  const { scale, translate } = transform;
  const decodedArcs = arcs.map((arc) => {
    let x = 0;
    let y = 0;
    return arc.map(([dx, dy]) => {
      x += dx;
      y += dy;
      return [x * scale[0] + translate[0], y * scale[1] + translate[1]];
    });
  });

  const ringFromArcIndexes = (indexes) => {
    const ring = [];
    for (const index of indexes) {
      const reverse = index < 0;
      const points = decodedArcs[reverse ? ~index : index];
      const sequence = reverse ? [...points].reverse() : points;
      const start = ring.length === 0 ? 0 : 1;
      for (let i = start; i < sequence.length; i += 1) ring.push(sequence[i]);
    }
    return ring;
  };

  const polygons = [];
  for (const geometry of topology.objects.land.geometries) {
    const groups = geometry.type === 'Polygon' ? [geometry.arcs] : geometry.arcs;
    for (const polygon of groups) {
      const outer = ringFromArcIndexes(polygon[0]);
      if (outer.length >= 4) polygons.push(outer);
    }
  }
  return polygons;
}

function project(lon, lat) {
  const x = ((lon + 180) / 360) * 1000;
  const y = ((90 - lat) / 180) * 500;
  return [Math.round(x * 10) / 10, Math.round(y * 10) / 10];
}

function simplify(ring, tolerance) {
  if (ring.length <= 8) return ring;
  const sq = tolerance * tolerance;
  const keep = new Uint8Array(ring.length);
  keep[0] = 1;
  keep[ring.length - 1] = 1;
  const stack = [[0, ring.length - 1]];
  while (stack.length > 0) {
    const [start, end] = stack.pop();
    const a = ring[start];
    const b = ring[end];
    let max = 0;
    let index = start;
    const dx = b[0] - a[0];
    const dy = b[1] - a[1];
    const length = dx * dx + dy * dy;
    for (let i = start + 1; i < end; i += 1) {
      const p = ring[i];
      let dist;
      if (length === 0) {
        const ox = p[0] - a[0];
        const oy = p[1] - a[1];
        dist = ox * ox + oy * oy;
      } else {
        let t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length;
        t = Math.max(0, Math.min(1, t));
        const qx = a[0] + t * dx - p[0];
        const qy = a[1] + t * dy - p[1];
        dist = qx * qx + qy * qy;
      }
      if (dist > max) {
        max = dist;
        index = i;
      }
    }
    if (max > sq) {
      keep[index] = 1;
      if (index - start > 1) stack.push([start, index]);
      if (end - index > 1) stack.push([index, end]);
    }
  }
  return ring.filter((_, index) => keep[index] === 1);
}

function pathFromRing(ring) {
  const points = ring.map(([lon, lat]) => project(lon, lat));
  if (points.length < 4) return null;
  let d = `M${points[0][0]} ${points[0][1]}`;
  for (let i = 1; i < points.length; i += 1) d += `L${points[i][0]} ${points[i][1]}`;
  return `${d}Z`;
}

async function packIpv4(csvPath, outPath) {
  const text = await readFile(csvPath, 'utf8');
  const countries = [];
  const countryIndex = new Map();
  const ranges = [];
  for (const line of text.split(/\n/u)) {
    const trimmed = line.trim();
    if (trimmed.length === 0) continue;
    const [startText, endText, country] = trimmed.split(',');
    if (!startText || !endText || !country || country.length !== 2) continue;
    const start = Number(startText);
    const end = Number(endText);
    if (!Number.isInteger(start) || !Number.isInteger(end) || start > end) continue;
    const code = country.toUpperCase();
    let index = countryIndex.get(code);
    if (index === undefined) {
      index = countries.length;
      countryIndex.set(code, index);
      countries.push(code);
    }
    const previous = ranges[ranges.length - 1];
    if (previous !== undefined && previous.country === index && previous.end + 1 >= start) {
      previous.end = Math.max(previous.end, end);
      continue;
    }
    ranges.push({ start, end, country: index });
  }

  const header = Buffer.alloc(16);
  header.write("HELX", 0, 4, "ascii");
  header.writeUInt16LE(1, 4);
  header.writeUInt16LE(countries.length, 6);
  header.writeUInt32LE(ranges.length, 8);
  header.writeUInt32LE(0, 12);
  const countryBlock = Buffer.from(`${countries.join('\n')}\n`, 'ascii');
  const table = Buffer.alloc(ranges.length * 9);
  for (let i = 0; i < ranges.length; i += 1) {
    const offset = i * 9;
    table.writeUInt32LE(ranges[i].start >>> 0, offset);
    table.writeUInt32LE(ranges[i].end >>> 0, offset + 4);
    table.writeUInt8(ranges[i].country, offset + 8);
  }
  await writeFile(outPath, Buffer.concat([header, countryBlock, table]));
  return { countries: countries.length, ranges: ranges.length, bytes: header.length + countryBlock.length + table.length };
}

async function packLand(jsonPath, outPath) {
  const topology = JSON.parse(await readFile(jsonPath, 'utf8'));
  const polygons = decodeTopojsonLand(topology);
  const paths = [];
  for (const ring of polygons) {
    const simplified = simplify(ring, 0.35);
    const d = pathFromRing(simplified);
    if (d !== null && d.length > 24) paths.push(d);
  }
  const source = `export const GLOBE_LAND_VIEWBOX = '0 0 1000 500';\n\nexport const GLOBE_LAND_PATHS: readonly string[] = ${JSON.stringify(paths)};\n`;
  await writeFile(outPath, source);
  return { polygons: paths.length, characters: source.length };
}

const ipv4 = await packIpv4(
  process.env.TEMP ? path.join(process.env.TEMP, 'geo-ipv4-num.csv') : '/tmp/geo-ipv4-num.csv',
  path.join(root, 'crates/helix-privd/data/ipv4-country.bin'),
);
const land = await packLand(
  process.env.TEMP ? path.join(process.env.TEMP, 'land-110m.json') : '/tmp/land-110m.json',
  path.join(root, 'frontend/src/globe-land.ts'),
);
console.log(JSON.stringify({ ipv4, land }));
