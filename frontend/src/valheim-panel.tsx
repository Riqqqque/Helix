import { useCallback, useEffect, useState } from 'preact/hooks';
import { ApiError, expectRecord, expectString } from './api';
import type { BrokerJob, NativeServerDetail } from './control-api';
import { CreateJobProgress } from './create-job-progress';
import { FileManager } from './file-manager';
import { useJobPolling } from './job-polling';
import { Icon } from './icons';
import { parseValheimMod, parseValheimStatus, valheimRequest, type ValheimMod, type ValheimSettings, type ValheimStatus } from './valheim-api';
import './valheim.css';

export const VALHEIM_MODIFIERS: Record<string, string[]> = {
  combat: ['veryeasy', 'easy', 'hard', 'veryhard'], deathpenalty: ['casual', 'veryeasy', 'easy', 'hard', 'hardcore'],
  resources: ['muchless', 'less', 'more', 'muchmore', 'most'], raids: ['none', 'muchless', 'less', 'more', 'muchmore'], portals: ['casual', 'hard', 'veryhard'],
};
const LABELS: Record<string, string> = { veryeasy: 'Very easy', veryhard: 'Very hard', deathpenalty: 'Death penalty', muchless: 'Much less', muchmore: 'Much more',
  nobuildcost: 'Free building', playerevents: 'Player-based raids', passivemobs: 'Passive enemies', nomap: 'No map' };
const modIcon = (mod: ValheimMod) => `https://gcdn.thunderstore.io/live/repository/icons/${encodeURIComponent(mod.package)}-${encodeURIComponent(mod.version)}.png`;
const label = (value: string) => LABELS[value] ?? value.charAt(0).toUpperCase() + value.slice(1);

export function ValheimSettingsFields({ value, onChange, disabled, creating = false }: {
  value: ValheimSettings; onChange: (settings: ValheimSettings) => void; disabled: boolean; creating?: boolean;
}) {
  const set = <K extends keyof ValheimSettings>(key: K, next: ValheimSettings[K]) => onChange({ ...value, [key]: next });
  return <div class="valheim-fields">
    <div class="form-grid">
      <label class="field"><span>World name</span><input value={value.world} maxlength={80} disabled={disabled} onInput={e => set('world', e.currentTarget.value)} /><small>Loads the matching world in worlds_local. Valheim 1.0 uses a whole world folder; older saves use a .db/.fwl pair. A new name creates a new world; it does not rename or delete the old one.</small></label>
      <label class="field"><span>Join password</span><input type="password" autocomplete="new-password" value={value.password} minlength={5} maxlength={128} disabled={disabled} onInput={e => set('password', e.currentTarget.value)} /><small>{creating ? 'Leave blank to generate one; find it in Valheim settings after creation. ' : ''}At least 5 characters; cannot be part of the server name.</small></label>
    </div>
    <label class="check-row"><input class="toggle-input" type="checkbox" checked={value.crossplay} disabled={disabled} onChange={e => set('crossplay', e.currentTarget.checked)} /><span><strong>Crossplay</strong><small>Steam, Xbox and Microsoft Store through PlayFab. Join by code or the server list, not a LAN or loopback IP. Console players cannot install client mods.</small></span></label>
    <label class="check-row"><input class="toggle-input" type="checkbox" checked={value.public} disabled={disabled} onChange={e => set('public', e.currentTarget.checked)} /><span><strong>Show in the community server list</strong><small>Visibility only. This does not change host firewall rules or forward your router.</small></span></label>
    <details class="valheim-options"><summary>World rules &amp; difficulty</summary>
      <p>Leave the preset unchanged to keep the world’s saved rules. Choosing a preset resets its modifiers on each start; the overrides below are applied after it. To clear old rules, choose Normal, then add your overrides.</p>
      <div class="form-grid">
        <label class="field"><span>Preset</span><select aria-label="Preset" disabled={disabled} value={value.preset} onChange={e => set('preset', e.currentTarget.value)}><option value="">Keep the world’s rules</option>{['normal', 'casual', 'easy', 'hard', 'hardcore', 'immersive', 'hammer'].map(p => <option key={p} value={p}>{label(p)}</option>)}</select></label>
        {Object.entries(VALHEIM_MODIFIERS).map(([key, values]) => <label key={key} class="field"><span>{label(key)}</span><select aria-label={label(key)} value={value.modifiers[key] ?? ''} disabled={disabled} onChange={e => {
          const modifiers = { ...value.modifiers }; if (e.currentTarget.value) modifiers[key] = e.currentTarget.value; else delete modifiers[key]; set('modifiers', modifiers);
        }}><option value="">Keep preset / world value</option>{values.map(v => <option key={v} value={v}>{label(v)}</option>)}</select></label>)}
      </div>
      {['nobuildcost', 'playerevents', 'passivemobs', 'nomap'].map(key => <label key={key} class="check-row"><input class="toggle-input" type="checkbox" disabled={disabled} checked={value.keys.includes(key)} onChange={e => set('keys', e.currentTarget.checked ? [...value.keys, key] : value.keys.filter(k => k !== key))} /><span>{label(key)}</span></label>)}
      <small>Unchecking a rule stops forcing it on; choose a preset to clear a rule already saved in the world.</small>
    </details>
    <details class="valheim-options"><summary>World saves &amp; automatic backups</summary>
      <p>These are Valheim’s own world backups. Helix’s full-server backups also include mods and configuration and can be restored from Backups.</p>
      <div class="form-grid">
        {([{ key: 'save_interval', title: 'Save interval (seconds)', min: 60, max: 86400 }, { key: 'backups', title: 'World backups to keep', min: 1, max: 100 }, { key: 'backup_short', title: 'Short backup interval (seconds)', min: 300, max: 604800 }, { key: 'backup_long', title: 'Long backup interval (seconds)', min: value.backup_short, max: 2592000 }] as const).map(f => <label key={f.key} class="field"><span>{f.title}</span><input type="number" min={f.min} max={f.max} value={value[f.key]} disabled={disabled} onInput={e => set(f.key, Number(e.currentTarget.value))} /></label>)}
      </div>
    </details>
  </div>;
}

export function ValheimPanel({ detail, csrfToken, canManage, onSessionExpired, onBackups, mode }: {
  detail: NativeServerDetail; csrfToken: string; canManage: boolean; onSessionExpired: () => void; onBackups: () => void; mode: 'settings' | 'mods';
}) {
  const [status, setStatus] = useState<ValheimStatus | null>(null);
  const [settings, setSettings] = useState<ValheimSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [reference, setReference] = useState('');
  const [preview, setPreview] = useState<ValheimMod | null>(null);
  const [updates, setUpdates] = useState<ValheimMod[] | null>(null);
  const [job, setJob] = useState<BrokerJob | null>(null);
  const [filePath, setFilePath] = useState<string | null>(null);
  const fail = useCallback((err: unknown) => {
    if (err instanceof ApiError && (err.status === 401 || err.code === 'csrf_rejected')) onSessionExpired();
    setError(err instanceof Error ? err.message : 'Valheim manager is unavailable');
  }, [onSessionExpired]);
  const load = useCallback(async (signal?: AbortSignal) => {
    try {
      const result = await valheimRequest(detail.id, csrfToken, { action: 'status' }, parseValheimStatus, signal);
      setStatus(result); setSettings(result.settings);
    } catch (err) { if (!signal?.aborted) fail(err); }
  }, [detail.id, csrfToken, fail]);
  useEffect(() => { const controller = new AbortController(); void load(controller.signal); return () => controller.abort(); }, [load]);
  const polling = useJobPolling({ job, csrfToken, onJob: setJob, onComplete: load, onSessionExpired });
  const active = busy || job?.status === 'queued' || job?.status === 'running';
  const stopped = detail.status !== 'online' && detail.status !== 'starting';
  const disabled = active || !canManage || !stopped || status?.runtime_current === false;
  const run = async (body: object) => {
    setBusy(true); setError(null); setNotice(null);
    try {
      const result = await valheimRequest(detail.id, csrfToken, body, v => expectRecord(v, 'Valheim'));
      const jobId = expectString(result, 'job_id', 'Valheim');
      setJob({ id: jobId, kind: 'valheim_manage', status: 'queued', stage: 'Queued', progressPercent: 0, createdAtUnixMs: Date.now(), updatedAtUnixMs: Date.now(), result: null, error: null });
    } catch (err) { fail(err); } finally { setBusy(false); }
  };
  const save = async () => {
    if (!settings || !status) return;
    setBusy(true); setError(null); setNotice(null);
    try {
      await valheimRequest(detail.id, csrfToken, { action: 'save_settings', expected_revision: status.expected_revision, settings }, v => expectRecord(v, 'Saved settings'));
      await load(); setNotice('Saved to valheim.json. These settings apply on the next start.');
    } catch (err) { fail(err); } finally { setBusy(false); }
  };
  const lookup = async () => {
    setBusy(true); setError(null); setPreview(null);
    try { setPreview(await valheimRequest(detail.id, csrfToken, { action: 'package', reference }, parseValheimMod)); }
    catch (err) { fail(err); } finally { setBusy(false); }
  };
  useEffect(() => {
    if (job?.status !== 'complete' || !job.result || typeof job.result !== 'object' || Array.isArray(job.result)) return;
    const result = job.result as Record<string, unknown>;
    if (Array.isArray(result.updates)) {
      try { setUpdates(result.updates.map(parseValheimMod)); } catch (err) { fail(err); }
    }
  }, [job, fail]);
  const files = (path: string) => setFilePath(`${detail.dataPath}/${path}`);
  if (filePath !== null) return <section class="valheim-panel"><button class="back-link" onClick={() => setFilePath(null)}>Back to Valheim {mode}</button><FileManager csrfToken={csrfToken} onSessionExpired={onSessionExpired} initialPath={filePath} /></section>;
  return <section class="server-tool valheim-panel" aria-label={`Valheim ${mode}`}>
    <div class="tool-head"><div><h2>{mode === 'settings' ? 'World & server settings' : 'Valheim mods'}</h2><p>{mode === 'settings' ? 'World rules, joining, saves and access lists.' : 'Install from Thunderstore, including required dependencies and BepInEx.'}</p></div><button class="button button--quiet" onClick={onBackups}><Icon name="backup" size={16} /> Backups &amp; restore</button></div>
    {!stopped && <p class="valheim-notice">Stop the server before saving settings or changing mods. Players can keep playing while you browse. Nothing restarts automatically.</p>}
    {status && !status.runtime_current && <p role="alert">This server uses an older Helix runtime. Stop it and update the runtime in Advanced before using these settings.</p>}
    {error && <p class="inline-error" role="alert">{error} <button class="button button--quiet" disabled={active} onClick={() => void load()}>Reload</button></p>}
    {notice && <p class="valheim-notice" role="status">{notice}</p>}
    {!status && !error && <p role="status">Loading Valheim configuration…</p>}
    {mode === 'settings' && settings && <>
      <ValheimSettingsFields value={settings} onChange={setSettings} disabled={disabled} />
      <div class="dialog-actions"><button class="button button--primary" disabled={disabled} onClick={() => void save()}>{busy ? 'Saving…' : 'Save for next start'}</button></div>
      <div class="valheim-tools">
        <article><Icon name="security" size={22} /><h3>Admins &amp; access</h3><p>Edit adminlist.txt, bannedlist.txt and permittedlist.txt in the server root. Use one Platform_UserID per line from the player’s F2 screen or server log. A non-empty permitted list blocks everyone else.</p><button class="button button--quiet" onClick={() => files('')}>Open access files</button></article>
        <article><Icon name="folder" size={22} /><h3>Worlds &amp; imports</h3><p>Valheim 1.0: import the entire world folder into worlds_local, including its chunk, .db2, .fwl2 and checkpoint files. Older worlds need both .db and .fwl. Stop and back up first; never copy just part of a world.</p><button class="button button--quiet" onClick={() => files('worlds_local')}>Open worlds</button></article>
        <article><Icon name="update" size={22} /><h3>Server software</h3><p>Steam’s current stable server build. Updates are manual, not on every boot. A verified full backup comes first; restore it if mods need the older build. Test a new game build against your client and mods.</p><div class="advanced-actions"><button class="button button--quiet" disabled={disabled} onClick={() => void run({ action: 'update_game', repair: false })}>Back up &amp; update</button><button class="button button--quiet" disabled={disabled} onClick={() => void run({ action: 'update_game', repair: true })}>Back up &amp; repair</button></div></article>
      </div>
    </>}
    {mode === 'mods' && <>
      <p class="valheim-notice">Review each mod’s requirements before installing. Some need matching client mods; Xbox cannot run those. Backups protect your files, but cannot guarantee third-party mod compatibility.</p>
      <form class="valheim-lookup" onSubmit={e => { e.preventDefault(); void lookup(); }}><label class="field"><span>Thunderstore package link or Author-Package-Version</span><input value={reference} placeholder="Paste a Valheim mod link" disabled={active} onInput={e => { setReference(e.currentTarget.value); setPreview(null); }} /></label><button class="button button--primary" disabled={active || !reference.trim()}>Preview</button><a class="button button--quiet" href="https://thunderstore.io/c/valheim/" target="_blank" rel="noopener noreferrer">Browse Thunderstore</a></form>
      {preview && <article class="valheim-mod-card"><img class="valheim-mod-image" src={modIcon(preview)} alt="" loading="lazy" referrerPolicy="no-referrer" onError={e => { e.currentTarget.hidden = true; }} /><div><h3>{preview.package.replaceAll('_', ' ')}</h3><span class="eyebrow">Version {preview.version}</span><p>{preview.description}</p>{preview.deprecated && <p role="alert">The author marked this package deprecated.</p>}<p>{preview.dependencies.length ? `Required: ${preview.dependencies.join(', ')}` : 'No declared dependencies. Helix adds the Valheim BepInEx runtime.'}</p></div><div class="advanced-actions"><a class="button button--quiet" href={preview.url} target="_blank" rel="noopener noreferrer">Read requirements</a><button class="button button--primary" disabled={disabled} onClick={() => void run({ action: 'install', reference: `${preview.package}-${preview.version}` })}>Back up &amp; install {preview.version}</button></div></article>}
      <div class="valheim-mod-heading"><h3>Installed packages · {status?.mods.length ?? 0}</h3><div class="advanced-actions"><button class="button button--quiet" disabled={active || !status?.mods.length} onClick={() => void run({ action: 'check_updates' })}>{busy ? 'Checking…' : 'Check mod updates'}</button><button class="button button--quiet" onClick={() => files('BepInEx/config')}>Edit mod configs</button><button class="button button--quiet" onClick={() => files('plugins')}>Upload local mods</button></div></div>
      {updates?.length === 0 && <p role="status">Installed versions are current on Thunderstore. Game compatibility still depends on each mod.</p>}
      {!status?.mods.length && <div class="valheim-empty"><Icon name="strands" size={28} /><h3>Start vanilla, add what you need</h3><p>Paste a mod link above. Helix resolves its dependencies, installs BepInEx, and keeps your existing configuration. Local DLLs go in plugins after BepInEx is installed.</p><button class="button button--quiet" disabled={disabled} onClick={() => void run({ action: 'install', reference: 'denikson-BepInExPack_Valheim' })}>Back up &amp; install BepInEx only</button></div>}
      {status?.mods.map(mod => <article class="valheim-mod-card" key={mod.package}><img class="valheim-mod-image" src={modIcon(mod)} alt="" loading="lazy" referrerPolicy="no-referrer" onError={e => { e.currentTarget.hidden = true; }} /><div><h3>{mod.package.replaceAll('_', ' ')}</h3><p>{mod.description}</p><small>{mod.version} · {mod.enabled ? 'Enabled' : 'Disabled'}{mod.dependencies.length ? ` · ${mod.dependencies.length} dependencies` : ''}</small></div><div class="advanced-actions">{updates?.find(item => item.package === mod.package) && <button class="button button--primary" disabled={disabled} onClick={() => void run({ action: 'install', reference: `${mod.package}-${updates.find(item => item.package === mod.package)?.version}` })}>Back up &amp; update</button>}<button class="button button--quiet" disabled={disabled} onClick={() => void run({ action: 'set_mod_enabled', package: mod.package, enabled: !mod.enabled })}>{mod.enabled ? 'Disable' : 'Enable'}</button><button class="button button--danger-quiet" disabled={disabled} onClick={() => { if (window.confirm(`Back up and remove ${mod.package}? World data and mod configuration will be kept. Required dependencies cannot be removed.`)) void run({ action: 'remove_mod', package: mod.package }); }}>Remove</button></div></article>)}
      <p class="valheim-footnote">Every mod change creates a full server backup. Packages are staged before activation; configs are kept in BepInEx/config. Removing a mod does not remove its world data or configuration. Restore a full backup to undo world changes made after playing with a mod.</p>
    </>}
    {job && <CreateJobProgress job={job} copy="Jobs continue if you refresh. File changes require a stopped server and a safety backup." />}
    {polling.error && <p role="alert">{polling.error} <button class="button button--quiet" onClick={polling.resume}>Resume checking</button></p>}
  </section>;
}
