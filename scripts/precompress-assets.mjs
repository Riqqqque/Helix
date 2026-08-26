import { brotliCompress, gzip, constants as zlibConstants } from 'node:zlib';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { lstat, readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

const compressBrotli = promisify(brotliCompress);
const compressGzip = promisify(gzip);
const root = fileURLToPath(new URL('../frontend/dist/', import.meta.url));
const compressibleExtensions = new Set([
  '.css',
  '.html',
  '.js',
  '.json',
  '.svg',
  '.txt',
  '.webmanifest',
  '.xml',
]);
const maximumFiles = 10_000;

let filesSeen = 0;
let filesCompressed = 0;
let rawBytes = 0;
let gzipBytes = 0;
let brotliBytes = 0;

async function walk(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name, 'en'));

  for (const entry of entries) {
    const target = path.join(directory, entry.name);
    const metadata = await lstat(target);
    if (metadata.isSymbolicLink()) {
      throw new Error(`refusing to precompress a symbolic link: ${target}`);
    }
    if (metadata.isDirectory()) {
      await walk(target);
      continue;
    }
    if (!metadata.isFile()) {
      throw new Error(`refusing to precompress a special file: ${target}`);
    }
    filesSeen += 1;
    if (filesSeen > maximumFiles) {
      throw new Error(`frontend output exceeds the ${maximumFiles}-file safety limit`);
    }
    if (!compressibleExtensions.has(path.extname(entry.name).toLowerCase())) {
      continue;
    }

    const source = await readFile(target);
    const [gzipOutput, brotliOutput] = await Promise.all([
      compressGzip(source, { level: 9 }),
      compressBrotli(source, {
        params: {
          [zlibConstants.BROTLI_PARAM_QUALITY]: 11,
          [zlibConstants.BROTLI_PARAM_SIZE_HINT]: source.length,
        },
      }),
    ]);
    await Promise.all([
      writeFile(`${target}.gz`, gzipOutput),
      writeFile(`${target}.br`, brotliOutput),
    ]);
    filesCompressed += 1;
    rawBytes += source.length;
    gzipBytes += gzipOutput.length;
    brotliBytes += brotliOutput.length;
  }
}

await walk(root);
process.stdout.write(
  `Precompressed ${filesCompressed} frontend files: raw=${rawBytes} gzip=${gzipBytes} brotli=${brotliBytes} bytes\n`,
);
