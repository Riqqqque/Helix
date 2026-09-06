import { useState } from 'preact/hooks';
import { ApiError } from './api';
import { changeNativeRuntime, getMinecraftVersions, type BrokerJob, type MinecraftSoftware, type NativeServerDetail } from './control-api';
import { CreateJobProgress } from './create-job-progress';
import { useJobPolling } from './job-polling';
import './server-runtime.css';

export function runtimeSoftware(value: string): MinecraftSoftware | null {
  const software = value.toLowerCase();
  switch (software) {
    case 'vanilla': case 'paper': case 'purpur': case 'folia': case 'leaves': case 'fabric': case 'neoforge': case 'forge': case 'quilt': case 'pumpkin': case 'pufferfish': return software;
    default: return null;
  }
}

export function ServerRuntimeControls({ detail, csrfToken, canManage, onComplete, onSessionExpired, onBackups }: {
  detail: NativeServerDetail; csrfToken: string; canManage: boolean;
  onComplete: () => void | Promise<void>; onSessionExpired: () => void; onBackups: () => void;
}) {
  const [mode, setMode] = useState<'repair' | 'version'>('repair');
  const [versions, setVersions] = useState<string[]>([]);
  const [selected, setSelected] = useState('');
  const [busy, setBusy] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [job, setJob] = useState<BrokerJob | null>(null);
  const polling = useJobPolling({ job, csrfToken, onJob: setJob, onComplete, onSessionExpired });
  const software = runtimeSoftware(detail.software);
  if (detail.kind !== 'minecraft' || software === null) return null;
  const active = busy || job?.status === 'queued' || job?.status === 'running';
  const fail = (err: unknown) => {
    if (err instanceof ApiError && (err.status === 401 || err.code === 'csrf_rejected')) onSessionExpired();
    setError(err instanceof Error ? err.message : 'Could not change this runtime.');
  };
  const loadVersions = async () => {
    setMode('version'); setConfirmed(false); setBusy(true); setError(null);
    try {
      const catalog = await getMinecraftVersions(software, csrfToken);
      setVersions(catalog.versions);
      setSelected(catalog.versions.includes(detail.minecraftVersion) ? detail.minecraftVersion : (catalog.latestVersion ?? catalog.versions[0] ?? ''));
    } catch (err) { fail(err); } finally { setBusy(false); }
  };
  const submit = async () => {
    setBusy(true); setError(null);
    try {
      const dispatch = await changeNativeRuntime(detail, mode === 'repair' ? null : selected, csrfToken);
      if (!dispatch.jobId) throw new Error('The runtime job could not be tracked. Check Background activity before retrying.');
      setJob({ id: dispatch.jobId, kind: 'server_runtime', status: 'queued', stage: 'Queued', progressPercent: 0, createdAtUnixMs: Date.now(), updatedAtUnixMs: Date.now(), result: null, error: null });
      setConfirmed(false);
    } catch (err) { fail(err); } finally { setBusy(false); }
  };
  return <section class="runtime-controls" aria-label="Server software versions">
    <div><h3>Server software</h3><p>{detail.software} · Minecraft {detail.minecraftVersion} · Build {detail.build}</p></div>
    <div class="advanced-actions">
      <button class="button button--quiet" disabled={active || !canManage} aria-pressed={mode === 'repair'} onClick={() => { setMode('repair'); setConfirmed(false); }}>Repair current runtime</button>
      {detail.modpack === null && <button class="button button--quiet" disabled={active || !canManage} aria-pressed={mode === 'version'} onClick={() => void loadVersions()}>Choose version / update build</button>}
      <button class="button button--quiet" onClick={onBackups}>Backups &amp; restore</button>
    </div>
    <p>{mode === 'repair' ? 'Re-download the exact installed runtime and rebuild its loader libraries. Worlds, mods, plugins and settings are kept. This does not repair broken mods or damaged worlds.' : 'Choose a published version. Helix installs its latest supported build and required Java runtime. Check your mods and plugins first. Downgrades require a matching full backup or a separate server.'}</p>
    {detail.modpack !== null && <p>Modpack Minecraft and loader versions stay pinned. Use the modpack update controls to upgrade the whole pack.</p>}
    {mode === 'version' && <label>Published {software === 'pumpkin' ? 'Pumpkin release' : 'Minecraft version'}<select disabled={active} value={selected} onChange={(event) => { setSelected(event.currentTarget.value); setConfirmed(false); }}>
      <option value="">Choose a version</option>{versions.map((version) => <option key={version} value={version}>{version}</option>)}
    </select></label>}
    <p>A full safety backup is required before files change. Running servers restart and are checked; failed startup restores that backup. Stopped servers stay stopped and are not startup-tested—use Backups to restore if the next start fails.</p>
    <label class="runtime-confirm"><input type="checkbox" checked={confirmed} disabled={active || !canManage} onChange={(event) => setConfirmed(event.currentTarget.checked)} /> Back up {detail.name} and apply this change.</label>
    <button class="button" disabled={active || !canManage || !confirmed || (mode === 'version' && !selected)} onClick={() => void submit()}>{busy ? 'Working…' : mode === 'repair' ? 'Back up & repair' : 'Back up & change version'}</button>
    {error && <p role="alert">{error}</p>}
    {job && <CreateJobProgress job={job} copy="This runs on the server. Refreshing or closing this page does not cancel it." />}
    {polling.error && <p role="alert">{polling.error} <button class="button button--quiet" onClick={polling.resume}>Resume checking</button></p>}
  </section>;
}
