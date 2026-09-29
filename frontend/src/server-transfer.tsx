import { useMemo, useState } from 'preact/hooks';
import { ApiError, expectRecord, requestJson } from './api';
import type { BrokerJob, ManagedServer, NativeServerDetail } from './control-api';
import { CreateJobProgress } from './create-job-progress';
import { formatBytes } from './format';
import { Icon } from './icons';
import { useJobPolling } from './job-polling';
import './server-transfer.css';

export type TransferPart = 'plugins' | 'plugin_data' | 'mods' | 'configs' | 'server_properties' | 'datapacks' | 'player_lists' | 'worlds';

interface PartCopy { id: TransferPart; title: string; detail: string; caution?: boolean }

const PLUGIN_SOFTWARE = ['paper', 'purpur', 'folia', 'leaves', 'pufferfish', 'custom'];
const MOD_SOFTWARE = ['fabric', 'quilt', 'forge', 'neoforge', 'custom'];

/** The parts that make sense for this software, in the order people usually want them. */
export function transferParts(software: string): PartCopy[] {
  const key = software.toLowerCase();
  const parts: PartCopy[] = [];
  if (PLUGIN_SOFTWARE.includes(key)) {
    parts.push({ id: 'plugins', title: 'Plugins and their settings', detail: 'JARs and config files. Databases, logs, and player files stay on the target.' });
  }
  if (MOD_SOFTWARE.includes(key)) parts.push({ id: 'mods', title: 'Mods', detail: 'Every JAR in mods/.' });
  parts.push(
    { id: 'configs', title: 'Server configuration', detail: 'bukkit, spigot, paper and loader files, and config/.' },
    { id: 'datapacks', title: 'Datapacks', detail: 'The world’s datapacks folder.' },
    { id: 'server_properties', title: 'Gameplay settings', detail: 'server.properties values such as difficulty and PvP. Ports, world name, MOTD, and player limit stay.' },
    { id: 'player_lists', title: 'Whitelist, operators, and bans', detail: 'Replaces the target’s lists.', caution: true },
  );
  if (PLUGIN_SOFTWARE.includes(key)) {
    parts.push({ id: 'plugin_data', title: 'All plugin data', detail: 'Includes databases and player files. Replaces the target’s, such as its CoreProtect history.', caution: true });
  }
  parts.push({ id: 'worlds', title: 'Worlds', detail: 'Replaces the target’s world, nether, and end.', caution: true });
  return parts;
}

export function defaultTransferParts(software: string): TransferPart[] {
  const available = new Set(transferParts(software).map((part) => part.id));
  return (['plugins', 'mods', 'configs', 'datapacks'] as TransferPart[]).filter((part) => available.has(part));
}

interface Preflight {
  parts: Record<string, { files: number; bytes: number; items: string[] }>;
  totalBytes: number;
  errors: string[];
  warnings: string[];
  targetRunning: boolean;
}

function strings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === 'string').slice(0, 64) : [];
}

export function parseTransferPreflight(value: unknown): Preflight {
  const root = expectRecord(value, 'transfer check');
  const parts: Preflight['parts'] = {};
  const raw = root.parts !== null && typeof root.parts === 'object' ? root.parts as Record<string, unknown> : {};
  for (const [key, entry] of Object.entries(raw)) {
    if (entry === null || typeof entry !== 'object') continue;
    const record = entry as Record<string, unknown>;
    parts[key] = {
      files: typeof record.files === 'number' ? record.files : 0,
      bytes: typeof record.bytes === 'number' ? record.bytes : 0,
      items: strings(record.items),
    };
  }
  const target = root.target !== null && typeof root.target === 'object' ? root.target as Record<string, unknown> : {};
  return {
    parts,
    totalBytes: typeof root.total_bytes === 'number' ? root.total_bytes : 0,
    errors: strings(root.errors),
    warnings: strings(root.warnings),
    targetRunning: target.running === true,
  };
}

/** Helix Minecraft servers other than this one; AMP servers cannot receive files. */
export function transferTargets(servers: ManagedServer[], sourceId: string): ManagedServer[] {
  return servers.filter((server) => server.manager === 'helix' && server.kind === 'minecraft' && server.id !== sourceId);
}

export function ServerTransferCard({ detail, servers, csrfToken, canManage, onComplete, onSessionExpired }: {
  detail: NativeServerDetail; servers: ManagedServer[]; csrfToken: string; canManage: boolean;
  onComplete: () => void | Promise<void>; onSessionExpired: () => void;
}) {
  const targets = useMemo(() => transferTargets(servers, detail.id), [servers, detail.id]);
  const available = useMemo(() => transferParts(detail.software), [detail.software]);
  const [targetId, setTargetId] = useState(targets[0]?.id ?? '');
  const [parts, setParts] = useState<TransferPart[]>(() => defaultTransferParts(detail.software));
  const [removeMissing, setRemoveMissing] = useState(false);
  const [preflight, setPreflight] = useState<Preflight | null>(null);
  const [confirmName, setConfirmName] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [job, setJob] = useState<BrokerJob | null>(null);
  const polling = useJobPolling({ job, csrfToken, onJob: setJob, onComplete, onSessionExpired });
  if (detail.kind !== 'minecraft') return null;
  const target = targets.find((server) => server.id === targetId) ?? null;
  const active = busy || job?.status === 'queued' || job?.status === 'running';
  const fail = (err: unknown) => {
    if (err instanceof ApiError && (err.status === 401 || err.code === 'csrf_rejected')) onSessionExpired();
    setError(err instanceof Error ? err.message : 'The transfer could not be checked.');
  };
  const reset = () => { setPreflight(null); setConfirmName(''); setError(null); };
  const check = async () => {
    if (target === null || parts.length === 0) return;
    setBusy(true); reset();
    try {
      setPreflight(await requestJson(`/api/v1/servers/${encodeURIComponent(detail.id)}/transfer/preflight`, parseTransferPreflight, {
        method: 'POST', csrfToken, body: { target_id: target.id, parts }, timeoutMs: 120_000,
      }));
    } catch (err) { fail(err); } finally { setBusy(false); }
  };
  const start = async () => {
    if (target === null) return;
    setBusy(true); setError(null);
    try {
      const dispatch = await requestJson(`/api/v1/servers/${encodeURIComponent(detail.id)}/transfer`, (value) => {
        const record = expectRecord(value, 'transfer');
        return typeof record.job_id === 'string' ? record.job_id : null;
      }, { method: 'POST', csrfToken, body: { target_id: target.id, parts, confirmation_name: confirmName, remove_missing_jars: removeMissing } });
      if (dispatch === null) throw new Error('The transfer could not be tracked. Check Background activity before retrying.');
      setJob({ id: dispatch, kind: 'server_transfer', status: 'queued', stage: 'Queued', progressPercent: 0, createdAtUnixMs: Date.now(), updatedAtUnixMs: Date.now(), result: null, error: null });
    } catch (err) { fail(err); } finally { setBusy(false); }
  };
  const ready = preflight !== null && preflight.errors.length === 0 && target !== null && confirmName === target.name;
  const hasJars = parts.includes('plugins') || parts.includes('plugin_data') || parts.includes('mods');

  return <section class="server-transfer" aria-label="Copy to another server">
    <header class="server-transfer__head">
      <div>
        <h3>Copy to another server</h3>
        <p>Move a finished setup, for example from a test server to your public one. {detail.name} is only read.</p>
      </div>
    </header>
    {targets.length === 0 ? <p class="server-transfer__empty"><Icon name="info" size={14} />Create or import another Helix Minecraft server to copy to. AMP servers cannot receive files; import them into Helix first.</p> : <>
      <label class="server-transfer__target">
        <span>Copy to</span>
        <select value={targetId} disabled={active || !canManage} onChange={(event) => { setTargetId(event.currentTarget.value); reset(); }}>
          {targets.map((server) => <option key={server.id} value={server.id}>{server.name} · {server.software} {server.version}</option>)}
        </select>
      </label>
      <fieldset class="server-transfer__parts" disabled={active || !canManage}>
        <legend>What to copy</legend>
        {available.map((part) => <label key={part.id} class={`server-transfer__part${part.caution ? ' is-caution' : ''}`}>
          <input type="checkbox" checked={parts.includes(part.id)} onChange={() => { setParts((current) => current.includes(part.id) ? current.filter((id) => id !== part.id) : [...current, part.id]); reset(); }} />
          <span><strong>{part.title}</strong><small>{part.detail}</small></span>
        </label>)}
      </fieldset>
      {hasJars && <label class="server-transfer__option check-row">
        <input type="checkbox" checked={removeMissing} disabled={active || !canManage} onChange={(event) => { setRemoveMissing(event.currentTarget.checked); reset(); }} />
        <span><strong>Remove plugins or mods the source doesn’t have</strong><small>Makes the target’s JARs match exactly. Off: extra JARs on the target are kept.</small></span>
      </label>}
      <div class="server-transfer__actions">
        <button class="button button--quiet" type="button" disabled={active || !canManage || target === null || parts.length === 0} onClick={() => void check()}>{busy && preflight === null ? 'Checking…' : 'Check what will be copied'}</button>
      </div>
      {preflight !== null && <div class="server-transfer__review">
        <ul class="server-transfer__summary">
          {parts.map((part) => {
            const summary = preflight.parts[part];
            const title = available.find((entry) => entry.id === part)?.title ?? part;
            return <li key={part}><strong>{title}</strong><span>{summary === undefined ? '—' : summary.files === 0 ? 'Nothing to copy' : `${summary.files} file${summary.files === 1 ? '' : 's'} · ${formatBytes(summary.bytes)}`}</span>
              {summary !== undefined && summary.items.length > 0 && (part === 'plugins' || part === 'mods' || part === 'plugin_data') && <small>{summary.items.filter((item) => item.toLowerCase().endsWith('.jar')).slice(0, 12).join(', ')}</small>}
            </li>;
          })}
        </ul>
        {preflight.errors.map((message) => <p key={message} class="server-transfer__error" role="alert">{message}</p>)}
        {preflight.warnings.map((message) => <p key={message} class="server-transfer__warning"><Icon name="warning" size={13} />{message}</p>)}
        {preflight.errors.length === 0 && target !== null && <>
          <div class="server-transfer__safety"><Icon name="backup" size={15} /><p>Helix backs up {target.name} first{preflight.targetRunning ? ', stops it while copying, and starts it again. Players are disconnected for a minute or two' : ''}. If anything fails, the backup is restored.</p></div>
          <label class="server-transfer__confirm">
            <span>Type <strong>{target.name}</strong> to confirm</span>
            <input value={confirmName} disabled={active} autoComplete="off" spellcheck={false} onInput={(event) => setConfirmName(event.currentTarget.value)} />
          </label>
          <div class="server-transfer__actions">
            <button class="button button--primary" type="button" disabled={active || !ready} onClick={() => void start()}>{busy ? 'Starting…' : `Copy to ${target.name}`}</button>
          </div>
        </>}
      </div>}
    </>}
    {error !== null && <p class="server-transfer__error" role="alert">{error}</p>}
    {job !== null && <CreateJobProgress job={job} copy={job.status === 'complete' ? transferResultNote(job.result) : 'This runs on the server. Closing this page does not cancel it.'} />}
    {polling.error !== null && <p class="server-transfer__error" role="alert">{polling.error} <button class="button button--quiet" type="button" onClick={polling.resume}>Resume checking</button></p>}
  </section>;
}

export function transferResultNote(result: unknown): string {
  const record = result !== null && typeof result === 'object' ? result as Record<string, unknown> : {};
  const files = typeof record.files_copied === 'number' ? record.files_copied : 0;
  const replaced = Array.isArray(record.replaced_jars) ? record.replaced_jars.length : 0;
  const removed = Array.isArray(record.removed_jars) ? record.removed_jars.length : 0;
  const pieces = [`Copied ${files} file${files === 1 ? '' : 's'}.`];
  if (replaced > 0) pieces.push(`Replaced ${replaced} older JAR${replaced === 1 ? '' : 's'}.`);
  if (removed > 0) pieces.push(`Removed ${removed} JAR${removed === 1 ? '' : 's'} the source doesn’t have.`);
  pieces.push(record.restarted === true ? 'The server started again cleanly.' : 'Start the server when you are ready.');
  if (typeof record.backup_id === 'string') pieces.push('Its previous state is in Backups.');
  return pieces.join(' ');
}
