import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { InlineError, PageHead, ProgressBar, toneForPercent } from './dashboard-ui';
import { formatDuration, formatTimestamp } from './format';
import { Icon } from './icons';
import {
  createMachine,
  deleteMachine,
  getHubIdentity,
  listMachines,
  machineWebSocketUrl,
  powerMachine,
  probeMachine,
  refreshMachines,
  requestMachineTicket,
  updateMachine,
  wakeMachine,
  type HubIdentity,
  type Machine,
  type MachineAuthKind,
  type MachinePowerAction,
  type MachineProbe,
  type MachineUpsert,
} from './machines-api';
import { Dialog } from './modal';
import {
  TerminalSessionView,
  describeTerminalError,
  isExpiredSessionError,
  type TerminalSessionPhase,
} from './terminal-surface';
import './machines.css';

export interface MachinesPageProps {
  csrfToken: string;
  canView: boolean;
  canManage: boolean;
  onSessionExpired: () => void;
}

export function machineStatusTone(machine: Machine): 'good' | 'warn' | 'bad' | 'neutral' {
  const probe = machine.probe;
  if (probe === null) return 'neutral';
  if (probe.status === 'online') return 'good';
  if (probe.status === 'reachable' || probe.status === 'auth_failed' || probe.status === 'host_key') return 'warn';
  return 'bad';
}

export function machineStatusLabel(machine: Machine): string {
  const probe = machine.probe;
  if (probe === null) return 'Not probed';
  if (probe.status === 'online') return 'Online';
  if (probe.status === 'reachable') return 'Port open';
  if (probe.status === 'unreachable') return 'Unreachable';
  if (probe.status === 'auth_failed') return 'Auth failed';
  if (probe.status === 'host_key') return 'Host key mismatch';
  return 'Probe failed';
}

export function machineAddress(machine: Machine): string {
  return `${machine.username}@${machine.host}:${machine.port}`;
}

function authKindLabel(kind: MachineAuthKind): string {
  if (kind === 'key') return 'Helix hub key';
  if (kind === 'password') return 'Password prompt';
  return 'Host SSH config';
}

function loadPercent(probe: MachineProbe): number | null {
  if (probe.load === null || probe.cpuCount === null || probe.cpuCount === 0) return null;
  return Math.round((probe.load[0] / probe.cpuCount) * 100);
}

function memPercent(probe: MachineProbe): number | null {
  if (probe.memTotalBytes === null || probe.memAvailableBytes === null || probe.memTotalBytes === 0) return null;
  return Math.round(((probe.memTotalBytes - probe.memAvailableBytes) / probe.memTotalBytes) * 100);
}

function diskPercent(probe: MachineProbe): number | null {
  if (probe.diskTotalBytes === null || probe.diskAvailableBytes === null || probe.diskTotalBytes === 0) return null;
  return Math.round(((probe.diskTotalBytes - probe.diskAvailableBytes) / probe.diskTotalBytes) * 100);
}

function isSessionFailure(error: unknown): boolean {
  return isExpiredSessionError(error);
}

interface MachineDialogState {
  mode: 'create' | 'edit';
  machine: Machine | null;
}

export interface MachineFormDraft {
  label: string;
  host: string;
  port: string;
  username: string;
  authKind: MachineAuthKind;
  wolMac: string;
  notes: string;
}

function draftFor(machine: Machine | null): MachineFormDraft {
  if (machine === null) {
    return { label: '', host: '', port: '22', username: '', authKind: 'key', wolMac: '', notes: '' };
  }
  return {
    label: machine.label,
    host: machine.host,
    port: String(machine.port),
    username: machine.username,
    authKind: machine.authKind,
    wolMac: machine.wolMac ?? '',
    notes: machine.notes,
  };
}

export function draftToUpsert(draft: MachineFormDraft): MachineUpsert | string {
  const label = draft.label.trim();
  const host = draft.host.trim();
  const username = draft.username.trim();
  const port = Number.parseInt(draft.port, 10);
  if (label.length === 0 || label.length > 48) return 'Give the machine a label up to 48 characters.';
  if (host.length === 0 || host.length > 253) return 'Enter a host name or IP address.';
  if (!Number.isSafeInteger(port) || port < 1 || port > 65_535) return 'SSH port must be between 1 and 65535.';
  if (username.length === 0 || username.length > 64) return 'Enter the SSH username for this machine.';
  const wolMac = draft.wolMac.trim();
  if (wolMac.length > 0 && !/^[0-9a-fA-F]{2}([:-][0-9a-fA-F]{2}){5}$/.test(wolMac)) {
    return 'Wake-on-LAN needs a MAC address like 3c:52:82:ab:12:34.';
  }
  return {
    label,
    host,
    port,
    username,
    authKind: draft.authKind,
    notes: draft.notes.trim(),
    wolMac: wolMac === '' ? null : wolMac,
  };
}

export function MachinesPage({ csrfToken, canView, canManage, onSessionExpired }: MachinesPageProps) {
  const [machines, setMachines] = useState<Machine[]>([]);
  const [identity, setIdentity] = useState<HubIdentity | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [probing, setProbing] = useState<ReadonlySet<string>>(new Set());
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [dialog, setDialog] = useState<MachineDialogState | null>(null);
  const [deleting, setDeleting] = useState<Machine | null>(null);
  const [powerTarget, setPowerTarget] = useState<{ machine: Machine; action: MachinePowerAction } | null>(null);
  const [showIdentity, setShowIdentity] = useState(false);
  const [unlocked, setUnlocked] = useState<string | null>(null);
  const [tabs, setTabs] = useState<string[]>([]);
  const [activeTab, setActiveTab] = useState<string | null>(null);
  const [view, setView] = useState<'grid' | 'terminal'>('grid');
  const [tabPhases, setTabPhases] = useState<Record<string, TerminalSessionPhase>>({});
  const machinesRef = useRef<Machine[]>([]);
  machinesRef.current = machines;

  const applyProbe = useCallback((machineId: string, probe: MachineProbe, probedAtUnixMs: number): void => {
    setMachines((current) => current.map((machine) =>
      machine.id === machineId ? { ...machine, probe, probedAtUnixMs } : machine));
  }, []);

  const load = useCallback(async (signal?: AbortSignal): Promise<void> => {
    try {
      const [nextMachines, nextIdentity] = await Promise.all([
        listMachines(csrfToken, signal),
        getHubIdentity(csrfToken, signal).catch(() => null),
      ]);
      setMachines(nextMachines);
      setIdentity(nextIdentity);
      setError(null);
    } catch (nextError) {
      if (signal?.aborted === true) return;
      if (isSessionFailure(nextError)) onSessionExpired();
      else setError(describeTerminalError(nextError));
    } finally {
      if (signal?.aborted !== true) setLoading(false);
    }
  }, [csrfToken, onSessionExpired]);

  useEffect(() => {
    if (!canView) {
      setLoading(false);
      return;
    }
    const controller = new AbortController();
    void load(controller.signal);
    return () => controller.abort();
  }, [canView, load]);

  const openTerminal = useCallback((machine: Machine): void => {
    setTabs((current) => current.includes(machine.id) ? current : [...current, machine.id]);
    setActiveTab(machine.id);
    setView('terminal');
  }, []);

  const closeTab = useCallback((machineId: string): void => {
    setTabPhases((phases) => {
      const copy = { ...phases };
      delete copy[machineId];
      return copy;
    });
    setTabs((current) => {
      if (!current.includes(machineId)) return current;
      const next = current.filter((id) => id !== machineId);
      setActiveTab((active) => {
        if (active !== machineId) return active;
        if (next.length === 0) return null;
        const index = Math.min(current.indexOf(machineId), next.length - 1);
        return next[index] ?? null;
      });
      return next;
    });
  }, []);

  const handlePhase = useCallback((machineId: string, phase: TerminalSessionPhase): void => {
    setTabPhases((current) => ({ ...current, [machineId]: phase }));
  }, []);

  const probeOne = useCallback(async (machine: Machine): Promise<void> => {
    setProbing((current) => new Set(current).add(machine.id));
    setNotice(null);
    try {
      const probe = await probeMachine(machine.id, csrfToken);
      applyProbe(machine.id, probe, Date.now());
    } catch (nextError) {
      if (isSessionFailure(nextError)) onSessionExpired();
      else setNotice(describeTerminalError(nextError));
    } finally {
      setProbing((current) => {
        const next = new Set(current);
        next.delete(machine.id);
        return next;
      });
    }
  }, [csrfToken, applyProbe, onSessionExpired]);

  const probeAll = useCallback(async (): Promise<void> => {
    setRefreshing(true);
    setNotice(null);
    try {
      const results = await refreshMachines(csrfToken);
      const now = Date.now();
      for (const result of results) applyProbe(result.id, result.probe, now);
    } catch (nextError) {
      if (isSessionFailure(nextError)) onSessionExpired();
      else setNotice(describeTerminalError(nextError));
    } finally {
      setRefreshing(false);
    }
  }, [csrfToken, applyProbe, onSessionExpired]);

  const wake = useCallback(async (machine: Machine): Promise<void> => {
    setBusyAction(`wake:${machine.id}`);
    setNotice(null);
    try {
      const result = await wakeMachine(machine.id, csrfToken);
      setNotice(result.detail);
    } catch (nextError) {
      if (isSessionFailure(nextError)) onSessionExpired();
      else setNotice(describeTerminalError(nextError));
    } finally {
      setBusyAction(null);
    }
  }, [csrfToken, onSessionExpired]);

  const saveMachine = useCallback(async (draft: MachineFormDraft): Promise<string | null> => {
    const upsert = draftToUpsert(draft);
    if (typeof upsert === 'string') return upsert;
    if (dialog === null) return null;
    try {
      if (dialog.mode === 'create') {
        const created = await createMachine(upsert, csrfToken);
        setMachines((current) => [...current, created]);
        setNotice(`Added ${created.label}. Authorize the hub key on it, then probe.`);
      } else if (dialog.machine !== null) {
        const updated = await updateMachine(dialog.machine.id, upsert, csrfToken);
        setMachines((current) => current.map((machine) => machine.id === updated.id ? updated : machine));
        setNotice(`Saved ${updated.label}.`);
      }
      setDialog(null);
      return null;
    } catch (nextError) {
      if (isSessionFailure(nextError)) onSessionExpired();
      return describeTerminalError(nextError);
    }
  }, [csrfToken, dialog, onSessionExpired]);

  const confirmDelete = useCallback(async (machine: Machine): Promise<void> => {
    try {
      await deleteMachine(machine.id, csrfToken);
      closeTab(machine.id);
      setMachines((current) => current.filter((entry) => entry.id !== machine.id));
      setNotice(`Removed ${machine.label} from the rack.`);
      setDeleting(null);
    } catch (nextError) {
      if (isSessionFailure(nextError)) onSessionExpired();
      else setNotice(describeTerminalError(nextError));
    }
  }, [csrfToken, closeTab, onSessionExpired]);

  const confirmPower = useCallback(async (machine: Machine, action: MachinePowerAction, confirmation: string): Promise<string | null> => {
    try {
      const result = await powerMachine(machine.id, action, confirmation, csrfToken);
      setNotice(result.detail);
      setPowerTarget(null);
      return null;
    } catch (nextError) {
      if (isSessionFailure(nextError)) onSessionExpired();
      return describeTerminalError(nextError);
    }
  }, [csrfToken, onSessionExpired]);

  if (!canView) {
    return (
      <div class="page">
        <PageHead title="Machines" detail="Rack machine registry and SSH terminals." />
        <section class="surface machines-empty"><Icon name="warning" size={24} /><p>This account is not allowed to view the rack machines.</p></section>
      </div>
    );
  }

  const tabMachines = tabs
    .map((id) => machines.find((machine) => machine.id === id))
    .filter((machine): machine is Machine => machine !== undefined);
  const connectedCount = Object.values(tabPhases).filter((phase) => phase === 'connected').length;
  const showingTerminal = view === 'terminal' && tabMachines.length > 0;

  return (
    <div class="page page--machines">
      <PageHead
        title="Machines"
        detail="Every box on the rack — probe, wake, power, and SSH from one place."
        actions={(
          <span class={`state-label state-label--${connectedCount > 0 ? 'good' : 'neutral'}`}>
            <span class={`status-dot ${connectedCount > 0 ? 'status-dot--good' : ''}`} />
            {connectedCount === 0 ? 'No sessions' : `${connectedCount} session${connectedCount === 1 ? '' : 's'} open`}
          </span>
        )}
      />
      <InlineError message={error ?? notice} />

      {!showingTerminal && (
        <>
          <div class="machines-toolbar">
            <div class="machines-toolbar-actions">
              {canManage && (
                <button class="button button--primary" type="button" onClick={() => setDialog({ mode: 'create', machine: null })}>
                  <Icon name="plus" size={14} />Add machine
                </button>
              )}
              <button class="button button--quiet" type="button" disabled={refreshing || machines.length === 0} onClick={() => void probeAll()}>
                <Icon name="refresh" size={14} />{refreshing ? 'Probing…' : 'Probe all'}
              </button>
              <button class={`button button--quiet${showIdentity ? ' is-active' : ''}`} type="button" onClick={() => setShowIdentity((current) => !current)}>
                <Icon name="security" size={14} />Hub identity
              </button>
              {tabs.length > 0 && (
                <button class="button button--quiet" type="button" onClick={() => { setActiveTab((active) => active ?? tabs[0] ?? null); setView('terminal'); }}>
                  <Icon name="terminal" size={14} />Sessions ({tabs.length})
                </button>
              )}
              {unlocked !== null && (
                <button class="button button--quiet" type="button" onClick={() => setUnlocked(null)}>
                  <Icon name="security" size={14} />Lock terminals
                </button>
              )}
            </div>
          </div>

          {showIdentity && <IdentityPanel identity={identity} />}

          {loading ? (
            <div class="detail-loading" aria-busy="true"><Icon name="servers" size={28} /><span>Loading the rack…</span></div>
          ) : machines.length === 0 ? (
            <section class="surface machines-empty">
              <Icon name="servers" size={28} />
              <strong>No machines registered yet</strong>
              <p>Add each box on the rack once — Helix keeps its own SSH keypair and known-hosts so machines stay isolated from your personal SSH config.</p>
              {canManage && <button class="button button--primary" type="button" onClick={() => setDialog({ mode: 'create', machine: null })}><Icon name="plus" size={14} />Add the first machine</button>}
            </section>
          ) : (
            <div class="machines-grid">
              {machines.map((machine) => (
                <MachineCard
                  key={machine.id}
                  machine={machine}
                  canManage={canManage}
                  probing={probing.has(machine.id)}
                  waking={busyAction === `wake:${machine.id}`}
                  tabOpen={tabs.includes(machine.id)}
                  onSsh={() => openTerminal(machine)}
                  onProbe={() => void probeOne(machine)}
                  onWake={() => void wake(machine)}
                  onEdit={() => setDialog({ mode: 'edit', machine })}
                  onDelete={() => setDeleting(machine)}
                  onPower={(action) => setPowerTarget({ machine, action })}
                />
              ))}
            </div>
          )}
        </>
      )}

      {showingTerminal && (
        <div class="machines-terminal">
          <div class="machines-tabstrip" role="tablist" aria-label="Open SSH sessions">
            <button class="machines-back" type="button" onClick={() => setView('grid')}>
              <Icon name="back" size={14} />Machines
            </button>
            {tabMachines.map((machine) => {
              const phase = tabPhases[machine.id];
              return (
                <button
                  key={machine.id}
                  type="button"
                  role="tab"
                  aria-selected={activeTab === machine.id}
                  class={`machine-tab${activeTab === machine.id ? ' is-active' : ''}`}
                  onClick={() => setActiveTab(machine.id)}
                >
                  <span class={`status-dot status-dot--${phase === 'connected' ? 'good' : phase === 'connecting' || phase === 'authorizing' ? 'busy' : 'neutral'}`} />
                  <span class="machine-tab-label">{machine.label}</span>
                  <span
                    class="machine-tab-close"
                    role="button"
                    aria-label={`Close ${machine.label} session`}
                    onClick={(event) => {
                      event.stopPropagation();
                      closeTab(machine.id);
                    }}
                  >
                    <Icon name="close" size={12} />
                  </span>
                </button>
              );
            })}
            {unlocked !== null && (
              <button class="button button--quiet machines-lock" type="button" onClick={() => setUnlocked(null)}>
                <Icon name="security" size={13} />Lock
              </button>
            )}
          </div>
          {tabMachines.map((machine) => (
            <div
              key={machine.id}
              class="machines-session"
              hidden={activeTab !== machine.id}
            >
              <TerminalSessionView
                title={`${machine.label} · ${machine.username}@${machine.host}`}
                ariaLabel={`SSH session for ${machine.label}`}
                password={unlocked}
                onUnlock={setUnlocked}
                onUnlockRejected={() => setUnlocked(null)}
                onSessionExpired={onSessionExpired}
                requestTicket={(password, columns, rows) => requestMachineTicket(machine.id, password, columns, rows, csrfToken)}
                webSocketUrl={machineWebSocketUrl}
                active={showingTerminal && activeTab === machine.id}
                onPhaseChange={(phase) => handlePhase(machine.id, phase)}
                onClose={() => closeTab(machine.id)}
                lockHeading={`Unlock ${machine.label}`}
                lockDetail={unlocked === null
                  ? 'Enter the current Helix dashboard password once — every open session on this page uses it.'
                  : 'Opening session…'}
              />
            </div>
          ))}
        </div>
      )}

      {dialog !== null && (
        <MachineDialog
          state={dialog}
          onClose={() => setDialog(null)}
          onSave={saveMachine}
        />
      )}
      {deleting !== null && (
        <Dialog title={`Remove ${deleting.label}?`} onClose={() => setDeleting(null)}>
          <p>Helix forgets this machine’s registry entry and its cached probe data. The hub key stays on the machine until you remove it from <code>authorized_keys</code> there.</p>
          <div class="dialog-actions">
            <button class="button button--quiet" type="button" onClick={() => setDeleting(null)}>Keep</button>
            <button class="button button--danger" type="button" onClick={() => void confirmDelete(deleting)}>Remove machine</button>
          </div>
        </Dialog>
      )}
      {powerTarget !== null && (
        <PowerDialog
          machine={powerTarget.machine}
          action={powerTarget.action}
          onClose={() => setPowerTarget(null)}
          onConfirm={confirmPower}
        />
      )}
    </div>
  );
}

function MachineCard({
  machine,
  canManage,
  probing,
  waking,
  tabOpen,
  onSsh,
  onProbe,
  onWake,
  onEdit,
  onDelete,
  onPower,
}: {
  machine: Machine;
  canManage: boolean;
  probing: boolean;
  waking: boolean;
  tabOpen: boolean;
  onSsh: () => void;
  onProbe: () => void;
  onWake: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onPower: (action: MachinePowerAction) => void;
}) {
  const tone = machineStatusTone(machine);
  const probe = machine.probe;
  const load = probe === null ? null : loadPercent(probe);
  const memory = probe === null ? null : memPercent(probe);
  const disk = probe === null ? null : diskPercent(probe);
  return (
    <article class={`machine-card surface machine-card--${tone}`}>
      <header class="machine-card-head">
        <span class={`status-dot status-dot--${tone === 'good' ? 'good' : tone === 'bad' ? 'bad' : tone === 'warn' ? 'busy' : 'neutral'}`} />
        <div class="machine-card-title">
          <strong>{machine.label}</strong>
          <span class="machine-card-address">{machineAddress(machine)}</span>
        </div>
        <span class={`machine-card-status machine-card-status--${tone}`}>{machineStatusLabel(machine)}</span>
      </header>
      {probe !== null && probe.status === 'online' && (
        <dl class="machine-card-facts">
          {probe.hostname !== null && <div><dt>Host</dt><dd>{probe.hostname}</dd></div>}
          {probe.os !== null && <div><dt>OS</dt><dd>{probe.os}</dd></div>}
          {probe.uptimeSeconds !== null && <div><dt>Uptime</dt><dd>{formatDuration(probe.uptimeSeconds)}</dd></div>}
          {probe.latencyMs !== null && <div><dt>SSH</dt><dd>{probe.latencyMs} ms</dd></div>}
        </dl>
      )}
      {probe !== null && probe.status !== 'online' && (
        <p class="machine-card-detail">{probe.detail}</p>
      )}
      {probe === null && (
        <p class="machine-card-detail">Not probed yet — check reachability or open a terminal.</p>
      )}
      {(load !== null || memory !== null || disk !== null) && (
        <div class="machine-card-metrics">
          {load !== null && <MetricBar label="Load" percent={load} />}
          {memory !== null && <MetricBar label="Mem" percent={memory} />}
          {disk !== null && <MetricBar label="Disk" percent={disk} />}
        </div>
      )}
      <p class="machine-card-meta">
        <span>{authKindLabel(machine.authKind)}</span>
        {probe?.containersRunning != null && <span>{probe.containersRunning} container{probe.containersRunning === 1 ? '' : 's'}</span>}
        {probe?.failedUnits != null && probe.failedUnits > 0 && <span class="machine-card-warn">{probe.failedUnits} failed unit{probe.failedUnits === 1 ? '' : 's'}</span>}
      </p>
      {machine.notes !== '' && <p class="machine-card-notes">{machine.notes}</p>}
      <footer class="machine-card-actions">
        <button class="button button--primary" type="button" onClick={onSsh}>
          <Icon name="terminal" size={14} />{tabOpen ? 'Terminal' : 'SSH'}
        </button>
        <button class="button button--quiet" type="button" disabled={probing} onClick={onProbe} aria-label={`Probe ${machine.label}`}>
          <Icon name="refresh" size={14} />
        </button>
        {machine.wolMac !== null && (
          <button class="button button--quiet" type="button" disabled={waking} onClick={onWake} aria-label={`Wake ${machine.label}`} title={`Wake-on-LAN ${machine.wolMac}`}>
            <Icon name="bell" size={14} />
          </button>
        )}
        {canManage && (
          <>
            <button class="button button--quiet" type="button" onClick={() => onPower('reboot')} aria-label={`Power actions for ${machine.label}`}>
              <Icon name="power" size={14} />
            </button>
            <button class="button button--quiet" type="button" onClick={onEdit} aria-label={`Edit ${machine.label}`}>
              <Icon name="edit" size={14} />
            </button>
            <button class="button button--quiet" type="button" onClick={onDelete} aria-label={`Remove ${machine.label}`}>
              <Icon name="trash" size={14} />
            </button>
          </>
        )}
      </footer>
      {machine.probedAtUnixMs !== null && (
        <span class="machine-card-probed">Checked {formatTimestamp(machine.probedAtUnixMs)}</span>
      )}
    </article>
  );
}

function MetricBar({ label, percent }: { label: string; percent: number }) {
  return (
    <div class="machine-metric">
      <span class="machine-metric-label">{label}</span>
      <ProgressBar value={percent} tone={toneForPercent(percent)} />
      <span class="machine-metric-value">{percent}%</span>
    </div>
  );
}

function IdentityPanel({ identity }: { identity: HubIdentity | null }) {
  const [copied, setCopied] = useState(false);
  const copyKey = (): void => {
    if (identity?.publicKey == null) return;
    void navigator.clipboard?.writeText(identity.publicKey).then(() => {
      setCopied(true);
      globalThis.setTimeout(() => setCopied(false), 2_000);
    });
  };
  return (
    <section class="surface machines-identity">
      <header><Icon name="security" size={18} /><strong>Hub SSH identity</strong></header>
      {identity === null ? (
        <p>The hub identity could not be loaded. Check that the privileged broker is running.</p>
      ) : !identity.available ? (
        <p>{identity.detail}</p>
      ) : (
        <>
          <p>Authorize this public key in <code>~{identity.user ?? 'user'}/.ssh/authorized_keys</code> on each machine, then Helix can probe and open terminals without passwords.</p>
          <div class="machines-identity-key">
            <code>{identity.publicKey}</code>
            <button class="button button--quiet" type="button" onClick={copyKey}>{copied ? 'Copied' : 'Copy'}</button>
          </div>
          <p class="machines-identity-fingerprint">Fingerprint <code>{identity.fingerprintSha256}</code> · runs as <code>{identity.user}</code> · dedicated known-hosts file</p>
        </>
      )}
    </section>
  );
}

function MachineDialog({
  state,
  onClose,
  onSave,
}: {
  state: MachineDialogState;
  onClose: () => void;
  onSave: (draft: MachineFormDraft) => Promise<string | null>;
}) {
  const [draft, setDraft] = useState<MachineFormDraft>(() => draftFor(state.machine));
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const update = (patch: Partial<MachineFormDraft>): void => setDraft((current) => ({ ...current, ...patch }));
  const submit = async (): Promise<void> => {
    setSaving(true);
    const failure = await onSave(draft);
    setSaving(false);
    if (failure !== null) setError(failure);
  };
  return (
    <Dialog title={state.mode === 'create' ? 'Add a rack machine' : `Edit ${state.machine?.label ?? 'machine'}`} onClose={onClose} wide>
      <div class="machine-form">
        <label><span>Label</span><input value={draft.label} maxLength={48} placeholder="builder, plex, nas…" onInput={(event) => update({ label: event.currentTarget.value })} /></label>
        <div class="machine-form-row">
          <label class="machine-form-grow"><span>Host or IP</span><input value={draft.host} maxLength={253} placeholder="192.0.2.10" onInput={(event) => update({ host: event.currentTarget.value })} /></label>
          <label class="machine-form-port"><span>SSH port</span><input value={draft.port} inputMode="numeric" maxLength={5} onInput={(event) => update({ port: event.currentTarget.value })} /></label>
        </div>
        <label><span>SSH username</span><input value={draft.username} maxLength={64} placeholder="operator" autoComplete="off" onInput={(event) => update({ username: event.currentTarget.value })} /></label>
        <fieldset class="machine-form-auth">
          <legend>Sign-in method</legend>
          <label><input type="radio" name="machine-auth" checked={draft.authKind === 'key'} onChange={() => update({ authKind: 'key' })} /><span><strong>Helix hub key</strong><small>Passwordless: probes, Wake, and power work. Add the hub public key to the machine’s authorized_keys.</small></span></label>
          <label><input type="radio" name="machine-auth" checked={draft.authKind === 'password'} onChange={() => update({ authKind: 'password' })} /><span><strong>Password prompt</strong><small>The remote SSH password prompt appears inside the terminal. Nothing is stored, so probes and power actions stay unavailable.</small></span></label>
          <label><input type="radio" name="machine-auth" checked={draft.authKind === 'system'} onChange={() => update({ authKind: 'system' })} /><span><strong>Host SSH config</strong><small>Use the terminal account’s own ~/.ssh keys and config on this host.</small></span></label>
        </fieldset>
        <label><span>Wake-on-LAN MAC <em>(optional)</em></span><input value={draft.wolMac} maxLength={17} placeholder="3c:52:82:ab:12:34" autoComplete="off" onInput={(event) => update({ wolMac: event.currentTarget.value })} /></label>
        <label><span>Notes <em>(optional)</em></span><textarea value={draft.notes} maxLength={2_048} rows={2} placeholder="Top rack, runs the build fleet…" onInput={(event) => update({ notes: event.currentTarget.value })} /></label>
        {error !== null && <p class="machine-form-error" role="alert">{error}</p>}
        <div class="dialog-actions">
          <button class="button button--quiet" type="button" onClick={onClose}>Cancel</button>
          <button class="button button--primary" type="button" disabled={saving} onClick={() => void submit()}>{saving ? 'Saving…' : state.mode === 'create' ? 'Add machine' : 'Save changes'}</button>
        </div>
      </div>
    </Dialog>
  );
}

function PowerDialog({
  machine,
  action,
  onClose,
  onConfirm,
}: {
  machine: Machine;
  action: MachinePowerAction;
  onClose: () => void;
  onConfirm: (machine: Machine, action: MachinePowerAction, confirmation: string) => Promise<string | null>;
}) {
  const [nextAction, setNextAction] = useState<MachinePowerAction>(action);
  const [confirmation, setConfirmation] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const submit = async (): Promise<void> => {
    setBusy(true);
    const failure = await onConfirm(machine, nextAction, confirmation);
    setBusy(false);
    if (failure !== null) setError(failure);
  };
  return (
    <Dialog title={`Power control for ${machine.label}`} onClose={onClose}>
      <div class="machine-form">
        <p>This runs <code>systemctl</code> over SSH on <strong>{machineAddress(machine)}</strong>. Anything running on the machine is interrupted.</p>
        <fieldset class="machine-form-auth">
          <legend>Action</legend>
          <label><input type="radio" name="machine-power" checked={nextAction === 'reboot'} onChange={() => setNextAction('reboot')} /><span><strong>Reboot</strong><small>Shut down cleanly and start again.</small></span></label>
          <label><input type="radio" name="machine-power" checked={nextAction === 'poweroff'} onChange={() => setNextAction('poweroff')} /><span><strong>Power off</strong><small>Shut down and stay off — use Wake-on-LAN or the PDU to bring it back.</small></span></label>
        </fieldset>
        <label><span>Type <code>{machine.label}</code> to confirm</span><input value={confirmation} maxLength={48} autoComplete="off" onInput={(event) => setConfirmation(event.currentTarget.value)} /></label>
        {error !== null && <p class="machine-form-error" role="alert">{error}</p>}
        <div class="dialog-actions">
          <button class="button button--quiet" type="button" onClick={onClose}>Cancel</button>
          <button class="button button--danger" type="button" disabled={busy || confirmation !== machine.label} onClick={() => void submit()}>{busy ? 'Sending…' : nextAction === 'reboot' ? 'Reboot machine' : 'Power off machine'}</button>
        </div>
      </div>
    </Dialog>
  );
}
