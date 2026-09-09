// Run against the local Vite dev server. API responses are isolated fixtures, never production.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { mkdir } from 'node:fs/promises';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.HELIX_PLAYWRIGHT ?? 'playwright');
const output = process.env.HELIX_BROWSER_OUTPUT ?? 'browser-results';
await mkdir(output, { recursive: true });
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));
let settings = { world: 'Dedicated', password: 'test-only-password', public: false, crossplay: false, save_interval: 1800, backups: 4, backup_short: 7200, backup_long: 43200, preset: '', modifiers: {}, keys: [] };
let revision = 'revision-1';
let mods = [];
const preview = { package: 'TestAuthor-TestMod', version: '1.0.0', description: 'A disposable package for browser verification.', dependencies: ['denikson-BepInExPack_Valheim-5.4.2350'], enabled: true, deprecated: false, url: 'https://thunderstore.io/c/valheim/p/TestAuthor/TestMod/' };
let pendingJob = null;
let jobResult = {};
let failSave = false;
let created = null;
await page.route('**/api/v1/**', async route => {
  const request = route.request();
  const body = request.method() === 'GET' ? {} : request.postDataJSON();
  let result;
  if (request.url().includes('/port-policies/valheim')) {
    result = { schema_version: 1, policy: { game: 'valheim', ranges: [{ start: 2456, end: 2465 }], ports: [], auto_forward_on_create: false }, capacity: 10, assigned_ports: [], amp_claimed_ports: [], available_count: 10, next_available_port: 2456 };
  } else if (request.url().endsWith('/servers/valheim')) {
    created = body; pendingJob = 'create-test'; result = { job_id: pendingJob };
  } else if (request.url().includes('/jobs/')) {
    result = { id: pendingJob, kind: 'valheim_manage', status: 'complete', stage: 'Saved', progress_percent: 100, created_at_unix_ms: Date.now(), updated_at_unix_ms: Date.now(), result: { backup_id: 'test-backup', ...jobResult }, error: null };
  } else if (body.action === 'status') {
    result = { settings, expected_revision: revision, mods, running: false, runtime_current: true };
  } else if (body.action === 'save_settings') {
    if (failSave || body.expected_revision !== revision) {
      await route.fulfill({ status: 409, json: { error: { code: 'conflict', message: 'Settings changed elsewhere; reload before saving' } } }); return;
    }
    settings = body.settings; revision += 'x'; result = { saved: true, settings, expected_revision: revision };
  } else if (body.action === 'package') {
    result = preview;
  } else if (body.action === 'install') {
    mods = [preview]; pendingJob = 'test-job-1'; result = { job_id: pendingJob };
  } else if (body.action === 'check_updates') {
    pendingJob = 'test-updates'; jobResult = { updates: [{ ...preview, version: '1.1.0' }] }; result = { job_id: pendingJob };
  } else if (body.action === 'set_mod_enabled') {
    mods[0] = { ...mods[0], enabled: body.enabled }; pendingJob = 'test-job-2'; result = { job_id: pendingJob };
  } else if (body.action === 'remove_mod') {
    mods = []; pendingJob = 'test-job-3'; result = { job_id: pendingJob };
  } else { throw new Error('Unmocked request: ' + request.url() + ' ' + JSON.stringify(body)); }
  await route.fulfill({ json: result });
});
try {
  const base = process.env.HELIX_BROWSER_URL ?? 'http://127.0.0.1:5176/e2e/valheim.html';
  await page.goto(base);
  await page.getByLabel('World name').fill('Browser world');
  await page.getByText('World rules & difficulty', { exact: true }).click();
  await page.getByLabel('Preset', { exact: true }).selectOption('hard');
  await page.getByLabel('Resources', { exact: true }).selectOption('more');
  await page.getByRole('button', { name: 'Save for next start' }).click();
  await page.getByRole('status').filter({ hasText: 'Saved to valheim.json' }).waitFor();
  assert.equal(settings.world, 'Browser world'); assert.equal(settings.modifiers.resources, 'more');
  await page.reload();
  await page.getByLabel('World name').waitFor();
  assert.equal(await page.getByLabel('World name').inputValue(), 'Browser world');
  await page.screenshot({ path: `${output}/valheim-settings-desktop.png`, fullPage: true });
  await page.getByLabel('Simulate running').check();
  assert.equal(await page.getByRole('button', { name: 'Save for next start' }).isDisabled(), true);
  await page.getByLabel('Simulate running').uncheck();
  failSave = true;
  await page.getByRole('button', { name: 'Save for next start' }).click();
  await page.getByRole('alert').waitFor();
  assert.equal(await page.getByLabel('World name').inputValue(), 'Browser world');
  failSave = false;
  await page.getByRole('button', { name: 'Mods', exact: true }).click();
  await page.getByLabel('Thunderstore package link or Author-Package-Version').fill('TestAuthor-TestMod-1.0.0');
  await page.getByRole('button', { name: 'Preview', exact: true }).click();
  await page.getByRole('button', { name: 'Back up & install 1.0.0' }).click();
  await page.getByRole('button', { name: 'Disable', exact: true }).waitFor({ state: 'visible', timeout: 10000 });
  await page.getByRole('button', { name: 'Check mod updates' }).click();
  await page.getByRole('button', { name: 'Back up & update', exact: true }).waitFor();
  await page.screenshot({ path: `${output}/valheim-mods-desktop.png`, fullPage: true });
  await page.getByRole('button', { name: 'Disable', exact: true }).click();
  await page.getByRole('button', { name: 'Enable', exact: true }).waitFor();
  page.once('dialog', dialog => dialog.accept());
  await page.getByRole('button', { name: 'Remove', exact: true }).click();
  await page.getByRole('heading', { name: 'Start vanilla, add what you need' }).waitFor();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: `${output}/valheim-mods-mobile.png`, fullPage: true });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true, 'mobile horizontal overflow');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByLabel('World name').waitFor();
  await page.screenshot({ path: `${output}/valheim-settings-mobile.png`, fullPage: true });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true, 'settings mobile horizontal overflow');
  await page.getByRole('button', { name: 'New Valheim server', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('Server name', { exact: true }).fill('Browser creation');
  await dialog.getByLabel('World name').fill('New world');
  await dialog.getByLabel('Join password').fill('test-only-password');
  await dialog.getByText('Crossplay', { exact: true }).click();
  await page.screenshot({ path: `${output}/valheim-create-mobile.png`, fullPage: true });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true, 'creation mobile horizontal overflow');
  await dialog.getByRole('button', { name: 'Create Valheim server', exact: true }).click();
  await page.waitForFunction(() => document.body.textContent.includes('Queued') || document.body.textContent.includes('Saved'));
  assert.equal(created.settings.world, 'New world');
  assert.equal(created.settings.crossplay, true);
  assert.equal(created.max_players, 10);
  assert.equal(created.network_exposure, 'private');
  assert.deepEqual(errors, []);
  console.log('Valheim browser checks passed: save/reload, conflict preservation, running lock, preview, install job, update discovery, disable/remove, desktop/mobile layouts; no page errors.');
} finally { await browser.close(); }
