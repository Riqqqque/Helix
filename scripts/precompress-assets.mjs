import { lstat, readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { constants as zlibConstants, brotliCompress, gzip } from 'node:zlib';
import { promisify } from 'node:util';

const compressBrotli = promisify(brotliCompress);
const compressGzip = promisify(gzip);
const defaultRoot = fileURLToPath(new URL('../frontend/dist/', import.meta.url));
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
const localOrigin = 'http://helix.local';

export const frontendBudgets = Object.freeze({
  initialGzipBytes: 75 * 1024,
  initialJavaScriptGzipBytes: 40 * 1024,
});

function attributeValue(tag, name) {
  const match = tag.match(new RegExp(`\\b${name}\\s*=\\s*(["'])(.*?)\\1`, 'iu'));
  return match?.[2] ?? null;
}

function localAssetPath(reference) {
  if (reference.startsWith('data:')) {
    return null;
  }

  let parsed;
  try {
    parsed = new URL(reference, `${localOrigin}/`);
  } catch {
    throw new Error(`invalid frontend asset URL in index.html: ${reference}`);
  }

  if (parsed.origin !== localOrigin || parsed.username !== '' || parsed.password !== '') {
    throw new Error(`initial frontend assets must remain same-origin: ${reference}`);
  }

  let decodedPath;
  try {
    decodedPath = decodeURIComponent(parsed.pathname);
  } catch {
    throw new Error(`invalid URL encoding in frontend asset path: ${reference}`);
  }

  const relativePath = path.posix.normalize(decodedPath.replace(/^\/+/, ''));
  if (relativePath === '.' || relativePath === '' || relativePath.startsWith('../')) {
    throw new Error(`frontend asset path escapes the build output: ${reference}`);
  }
  return relativePath;
}

export function collectInitialAssetPaths(indexHtml) {
  const assetPaths = new Set(['index.html']);
  const tags = indexHtml.match(/<(?:script|link)\b[^>]*>/giu) ?? [];

  for (const tag of tags) {
    const isScript = /^<script\b/iu.test(tag);
    if (!isScript) {
      const rel = attributeValue(tag, 'rel')?.toLowerCase().split(/\s+/u) ?? [];
      if (!rel.some((value) => ['modulepreload', 'preload', 'stylesheet'].includes(value))) {
        continue;
      }
    }

    const reference = attributeValue(tag, isScript ? 'src' : 'href');
    if (reference === null) {
      continue;
    }
    const assetPath = localAssetPath(reference);
    if (assetPath !== null) {
      assetPaths.add(assetPath);
    }
  }

  return [...assetPaths].sort((left, right) => left.localeCompare(right, 'en'));
}

export function measureInitialAssets(indexHtml, gzipBytesByPath) {
  const assetPaths = collectInitialAssetPaths(indexHtml);
  let initialGzipBytes = 0;
  let initialJavaScriptGzipBytes = 0;

  for (const assetPath of assetPaths) {
    const compressedBytes = gzipBytesByPath.get(assetPath);
    if (compressedBytes === undefined) {
      throw new Error(`initial frontend asset was not precompressed: ${assetPath}`);
    }
    initialGzipBytes += compressedBytes;
    if (path.posix.extname(assetPath).toLowerCase() === '.js') {
      initialJavaScriptGzipBytes += compressedBytes;
    }
  }

  return { assetPaths, initialGzipBytes, initialJavaScriptGzipBytes };
}

export function enforceFrontendBudgets(measurements, budgets = frontendBudgets) {
  if (measurements.initialGzipBytes > budgets.initialGzipBytes) {
    throw new Error(
      `initial frontend gzip budget exceeded: ${measurements.initialGzipBytes} > ${budgets.initialGzipBytes} bytes`,
    );
  }
  if (measurements.initialJavaScriptGzipBytes > budgets.initialJavaScriptGzipBytes) {
    throw new Error(
      `initial JavaScript gzip budget exceeded: ${measurements.initialJavaScriptGzipBytes} > ${budgets.initialJavaScriptGzipBytes} bytes`,
    );
  }
}

async function precompress(root) {
  let filesSeen = 0;
  let filesCompressed = 0;
  let rawBytes = 0;
  let gzipBytes = 0;
  let brotliBytes = 0;
  const gzipBytesByPath = new Map();

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
      gzipBytesByPath.set(path.relative(root, target).split(path.sep).join('/'), gzipOutput.length);
    }
  }

  await walk(root);
  const indexHtml = await readFile(path.join(root, 'index.html'), 'utf8');
  const initial = measureInitialAssets(indexHtml, gzipBytesByPath);
  enforceFrontendBudgets(initial);

  return { filesCompressed, rawBytes, gzipBytes, brotliBytes, ...initial };
}

async function main() {
  const result = await precompress(defaultRoot);
  process.stdout.write(
    `Precompressed ${result.filesCompressed} frontend files: raw=${result.rawBytes} gzip=${result.gzipBytes} brotli=${result.brotliBytes} bytes; ` +
      `initial-gzip=${result.initialGzipBytes}/${frontendBudgets.initialGzipBytes} bytes; ` +
      `initial-js-gzip=${result.initialJavaScriptGzipBytes}/${frontendBudgets.initialJavaScriptGzipBytes} bytes\n`,
  );
}

const invokedPath = process.argv[1] === undefined ? null : pathToFileURL(path.resolve(process.argv[1])).href;
if (invokedPath === import.meta.url) {
  await main();
}
