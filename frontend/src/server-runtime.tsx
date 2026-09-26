import { useState } from 'preact/hooks';
import { ApiError } from './api';
import { changeNativeRuntime, getMinecraftVersions, type BrokerJob, type MinecraftSoftware, type NativeServerDetail } from './control-api';
import { CreateJobProgress } from './create-job-progress';
import { Icon } from './icons';
import { useJobPolling } from './job-polling';
import './server-runtime.css';

export function runtimeSoftware(value: string): MinecraftSoftware | null {
  const software = value.toLowerCase();
  switch (software) {
    case 'vanilla': case 'paper': case 'purpur': case 'folia': case 'leaves': case 'fabric': case 'neoforge': case 'forge': case 'quilt': case 'pumpkin': case 'pufferfish': return software;
    default: return null;
  }
}

export type RuntimeMode = 'update' | 'version' | 'repair';

interface ModeCopy { title: string; summary: string; action: string }

/** What each option does, in plain words. Pumpkin tracks releases, not builds. */
export function runtimeModes(software: MinecraftSoftware, modpack: boolean): Array<{ id: RuntimeMode } & ModeCopy> {
  const modes: Array<{ id: RuntimeMode } & ModeCopy> = [];
  if (!modpack && software !== 'pumpkin') {
    modes.push({ id: 'update', title: 'Update build', summary: 'Install the newest build of the Minecraft version you already run. Worlds, plugins, mods and settings stay.', action: 'Back up & update' });
  }
  if (!modpack) {
    modes.push({ id: 'version', title: software === 'pumpkin' ? 'Change release' : 'Change version', summary: software === 'pumpkin' ? 'Move to a newer published Pumpkin release.' : 'Move to a newer Minecraft version. Check that your plugins and mods support it first. Downgrades are not possible here.', action: software === 'pumpkin' ? 'Back up & change release' : 'Back up & change version' });
  }
  modes.push({ id: 'repair', title: 'Repair files', summary: 'Re-download the exact version and build you have now and rebuild loader libraries. Use this if the server JAR was damaged or edited.', action: 'Back up & repair' });
  return modes;
}

export function ServerRuntimeControls({ detail, csrfToken, canManage, onComplete, onSessionExpired, onBackups }: {
  detail: NativeServerDetail; csrfToken: string; canManage: boolean;
  onComplete: () => void | Promise<void>; onSessionExpired: () => void; onBackups: () => void;
}) {
  const software = runtimeSoftware(detail.software);
  const modes = software === null ? [] : runtimeModes(software, detail.modpack !== null);
  const [mode, setMode] = useState<RuntimeMode>(modes[0]?.id ?? 'repair');
  const [versions, setVersions] = useState<string[] | null>(null);
  const [selected, setSelected] = useState('');
  const [busy, setBusy] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [job, setJob] = useState<BrokerJob | null>(null);
  const polling = useJobPolling({ job, csrfToken, onJob: setJob, onComplete, onSessionExpired });
  if (detail.kind !== 'minecraft' || software === null) return null;
  const active = busy || job?.status === 'queued' || job?.status === 'running';
  const current = modes.find((entry) => entry.id === mode) ?? modes[0] ?? { id: 'repair' as const, title: 'Repair files', summary: '', action: 'Back up & repair' };
  const fail = (err: unknown) => {
    if (err instanceof ApiError && (err.status === 401 || err.code === 'csrf_rejected')) onSessionExpired();
    setError(err instanceof Error ? err.message : 'Could not change this server software.');
  };
  const choose = async (next: RuntimeMode) => {
    setMode(next); setConfirmed(false); setError(null);
    if (next !== 'version' || versions !== null) return;
    setBusy(true);
    try {
      const catalog = await getMinecraftVersions(software, csrfToken);
      setVersions(catalog.versions);
      const newer = catalog.versions.filter((version) => version !== detail.minecraftVersion);
      setSelected(catalog.latestVersion !== null && catalog.latestVersion !== detail.minecraftVersion ? catalog.latestVersion : (newer[0] ?? ''));
    } catch (err) { fail(err); } finally { setBusy(false); }
  };
  const submit = async () => {
    setBusy(true); setError(null);
    try {
      const target = mode === 'repair' ? null : mode === 'update' ? detail.minecraftVersion : selected;
      const dispatch = await changeNativeRuntime(detail, target, csrfToken);
      if (!dispatch.jobId) throw new Error('The job could not be tracked. Check Background activity before retrying.');
      setJob({ id: dispatch.jobId, kind: 'server_runtime', status: 'queued', stage: 'Queued', progressPercent: 0, createdAtUnixMs: Date.now(), updatedAtUnixMs: Date.now(), result: null, error: null });
      setConfirmed(false);
    } catch (err) { fail(err); } finally { setBusy(false); }
  };
  const resultNote = job?.status === 'complete' && job.result !== null && typeof job.result === 'object'
    ? runtimeResultNote(job.result as Record<string, unknown>)
    : null;
  const needsVersion = mode === 'version' && selected === '';
  const running = detail.status !== 'stopped';
  return <section class="runtime-controls" aria-label="Server software">
    <header class="runtime-controls__head">
      <div>
        <h3>Server software</h3>
        <p><strong>{detail.software}</strong> · Minecraft {detail.minecraftVersion}{detail.build ? ` · build ${detail.build}` : ''} · Java {detail.javaVersion}</p>
      </div>
      <button class="button button--quiet" type="button" onClick={onBackups}><Icon name="backup" size={14} />Backups</button>
    </header>
    {detail.modpack !== null && <p class="runtime-controls__note"><Icon name="info" size={14} />This server runs the {detail.modpack.projectTitle} modpack, so its Minecraft and loader versions stay pinned together. Use <strong>Check for update</strong> above to update the whole pack.</p>}
    <div class="runtime-controls__modes" role="radiogroup" aria-label="What to do">
      {modes.map((entry) => (
        <label key={entry.id} class={`runtime-mode${mode === entry.id ? ' is-selected' : ''}`}>
          <input type="radio" name={`runtime-mode-${detail.id}`} value={entry.id} checked={mode === entry.id} disabled={active || !canManage} onChange={() => void choose(entry.id)} />
          <span><strong>{entry.title}</strong><small>{entry.summary}</small></span>
        </label>
      ))}
    </div>
    {mode === 'version' && (
      <label class="runtime-controls__version">
        <span>{software === 'pumpkin' ? 'Pumpkin release' : 'Minecraft version'}</span>
        <select disabled={active || versions === null} value={selected} onChange={(event) => { setSelected(event.currentTarget.value); setConfirmed(false); }}>
          {versions === null && <option value="">Loading versions…</option>}
          {versions !== null && <option value="">Choose a version</option>}
          {(versions ?? []).map((version) => <option key={version} value={version} disabled={version === detail.minecraftVersion}>{version}{version === detail.minecraftVersion ? ' (installed)' : ''}</option>)}
        </select>
      </label>
    )}
    <div class="runtime-controls__safety">
      <Icon name="backup" size={15} />
      <p>Helix makes a full backup first. {running ? 'The server restarts on the new files and is checked; if it does not start, Helix restores the backup automatically.' : 'The server stays stopped. If it does not start next time, restore the backup from Backups.'}</p>
    </div>
    <label class="runtime-confirm check-row">
      <input type="checkbox" checked={confirmed} disabled={active || !canManage} onChange={(event) => setConfirmed(event.currentTarget.checked)} />
      <span><strong>Back up {detail.name} and {current.title.toLowerCase()}</strong><small>{running ? 'Players are disconnected while it restarts.' : 'Nothing starts.'}</small></span>
    </label>
    <div class="runtime-controls__actions">
      <button class="button button--primary" type="button" disabled={active || !canManage || !confirmed || needsVersion} title={canManage ? undefined : 'Requires games.manage permission'} onClick={() => void submit()}>{busy ? 'Working…' : current.action}</button>
    </div>
    {error && <p class="runtime-controls__error" role="alert">{error}</p>}
    {job && <CreateJobProgress job={job} copy={resultNote ?? 'This runs on the server. Refreshing or closing this page does not cancel it.'} />}
    {polling.error && <p class="runtime-controls__error" role="alert">{polling.error} <button class="button button--quiet" type="button" onClick={polling.resume}>Resume checking</button></p>}
  </section>;
}

export function runtimeResultNote(result: Record<string, unknown>): string | null {
  if (result.already_current === true) return 'Already on the newest build. Nothing was changed and no backup was needed.';
  const version = typeof result.version === 'string' ? result.version : null;
  const build = typeof result.build === 'string' ? result.build : null;
  if (version === null) return null;
  const where = `Now on ${version}${build ? ` build ${build}` : ''}.`;
  return result.runtime_validation_performed === true ? `${where} The server started cleanly.` : `${where} Start the server when you are ready.`;
}
