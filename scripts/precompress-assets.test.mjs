import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  collectInitialAssetPaths,
  enforceFrontendBudgets,
  measureInitialAssets,
} from './precompress-assets.mjs';

describe('frontend asset budgets', () => {
  it('measures only first-route same-origin assets once', () => {
    const html = `
      <link rel="stylesheet" href="/assets/app.css?v=1">
      <link rel="modulepreload" href="/assets/shared.js">
      <link rel="icon" href="/icon.svg">
      <script type="module" src="/assets/app.js#entry"></script>
      <script type="module" src="/assets/app.js"></script>
    `;
    const sizes = new Map([
      ['index.html', 100],
      ['assets/app.css', 200],
      ['assets/shared.js', 300],
      ['assets/app.js', 400],
    ]);

    assert.deepEqual(collectInitialAssetPaths(html), [
      'assets/app.css',
      'assets/app.js',
      'assets/shared.js',
      'index.html',
    ]);
    assert.deepEqual(measureInitialAssets(html, sizes), {
      assetPaths: ['assets/app.css', 'assets/app.js', 'assets/shared.js', 'index.html'],
      initialGzipBytes: 1_000,
      initialJavaScriptGzipBytes: 700,
    });
  });

  it('rejects external initial assets and missing compressed output', () => {
    assert.throws(
      () => collectInitialAssetPaths('<script src="https://cdn.example/app.js"></script>'),
      /same-origin/u,
    );
    assert.throws(
      () => measureInitialAssets('<script src="/missing.js"></script>', new Map([['index.html', 10]])),
      /was not precompressed/u,
    );
  });

  it('fails independently when either founding budget is exceeded', () => {
    const budgets = { initialGzipBytes: 1_000, initialJavaScriptGzipBytes: 400 };
    assert.doesNotThrow(() =>
      enforceFrontendBudgets(
        { initialGzipBytes: 1_000, initialJavaScriptGzipBytes: 400 },
        budgets,
      ),
    );
    assert.throws(
      () => enforceFrontendBudgets(
        { initialGzipBytes: 1_001, initialJavaScriptGzipBytes: 100 },
        budgets,
      ),
      /frontend gzip budget exceeded/u,
    );
    assert.throws(
      () => enforceFrontendBudgets(
        { initialGzipBytes: 900, initialJavaScriptGzipBytes: 401 },
        budgets,
      ),
      /JavaScript gzip budget exceeded/u,
    );
  });
});
