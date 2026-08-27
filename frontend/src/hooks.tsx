import { useCallback, useEffect, useMemo, useState } from 'preact/hooks';
import { ApiError } from './api';
import { InlineError, PageHead } from './dashboard-ui';
import {
  getHookInventory,
  manageHookService,
  type HookConnection,
  type HookInventory,
  type HookServiceAction,
} from './hooks-api';
import { Icon, type IconName } from './icons';
import { InfoTip } from './info-tip';
import { Dialog } from './modal';
import './hooks.css';

interface HookDescriptor {
  name: string;
  category: string;
  summary: string;
  detail: string;
  icon: IconName;
  color: string;
  docs: string;
  setupSteps: string[];
}

const descriptors: Record<string, HookDescriptor> = {
  plex: {
    name: 'Plex',
    category: 'Media',
    summary: 'Media server health and lifecycle control',
    detail: 'Helix talks to the exact Plex systemd unit. Opening Plex still uses Plex Web, so library, playback, and account settings stay inside the interface Plex owns.',
    icon: 'play',
    color: '#e5a00d',
    docs: 'https://support.plex.tv/articles/200264746-quick-start-step-by-step-guides/',
    setupSteps: ['Install Plex Media Server for this Linux distribution.', 'Finish the Plex claim and library setup in Plex Web.', 'Refresh Hooks; Helix will detect the systemd service automatically.'],
  },
  amp: {
    name: 'AMP',
    category: 'Game hosting',
    summary: 'Imported game-server inventory and safe actions',
    detail: 'The AMP compatibility adapter keeps AMP separate. Helix imports verified instances and supported lifecycle actions, while AMP-only settings open in the real AMP panel.',
    icon: 'servers',
    color: '#6c8cff',
    docs: 'https://cubecoders.com/AMP',
    setupSteps: ['Install and configure AMP separately.', 'Add its loopback API endpoint and credentials to the protected Helix broker configuration.', 'Set the public panel port if it differs from the loopback API port.'],
  },
  tailscale: {
    name: 'Tailscale',
    category: 'Private networking',
    summary: 'Private remote access without publishing Helix',
    detail: 'Helix detects tailscaled and can control its exact service. Initial installation and tailnet sign-in stay guided because repository setup and identity approval differ by distribution and account.',
    icon: 'network',
    color: '#b9b9b9',
    docs: 'https://tailscale.com/docs/install/linux',
    setupSteps: ['Use the official Linux instructions for this distribution.', 'Run tailscale up and approve this host in the intended tailnet.', 'Return here and refresh; Helix will show service and boot state.'],
  },
  pterodactyl: {
    name: 'Pterodactyl Wings',
    category: 'Game hosting',
    summary: 'Node-agent health and lifecycle control',
    detail: 'This hook watches the Wings systemd service. A future authenticated panel adapter can add server inventory without weakening Pterodactyl’s ownership boundary.',
    icon: 'servers',
    color: '#7c9cff',
    docs: 'https://pterodactyl.io/wings/1.0/installing.html',
    setupSteps: ['Install a supported Pterodactyl panel and Wings node.', 'Complete the node configuration and verify Wings independently.', 'Refresh Hooks to pick up wings.service.'],
  },
  jellyfin: {
    name: 'Jellyfin',
    category: 'Media',
    summary: 'Media server service control and quick access',
    detail: 'Helix monitors and controls the exact Jellyfin systemd service. Media libraries and account administration remain in Jellyfin’s own web interface.',
    icon: 'play',
    color: '#a56dff',
    docs: 'https://jellyfin.org/docs/general/installation/linux/',
    setupSteps: ['Install Jellyfin using its supported package instructions.', 'Complete the first-run setup in Jellyfin Web.', 'Refresh Hooks to detect jellyfin.service.'],
  },
};

const actionLabels: Record<HookServiceAction, string> = {
  start: 'Start',
  stop: 'Stop',
  restart: 'Restart',
  enable: 'Start after boot',
  disable: 'Do not start after boot',
};

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : 'Helix could not complete that hook action.';
}

function descriptorFor(hook: HookConnection): HookDescriptor {
  return descriptors[hook.id] ?? {
    name: hook.id,
    category: 'Custom',
    summary: 'Configured host connection',
    detail: 'This connection was explicitly allowed in the protected Helix broker configuration.',
    icon: 'hooks',
    color: '#d7f64d',
    docs: '',
    setupSteps: ['Follow the integration documentation supplied with this hook.'],
  };
}

function panelHref(hook: HookConnection): string | null {
  if (typeof window === 'undefined') return null;
  const hostname = window.location.hostname;
  if (hook.id === 'amp' && hook.panelPort !== null) return `http://${hostname}:${hook.panelPort}/`;
  if (hook.id === 'plex') return `http://${hostname}:32400/web/`;
  if (hook.id === 'jellyfin') return `http://${hostname}:8096/`;
  if (hook.id === 'tailscale' && hook.installed) return 'https://login.tailscale.com/admin/machines';
  return null;
}

function statusLabel(hook: HookConnection): string {
  if (!hook.installed) return 'Available';
  if (hook.error !== null) return 'Needs attention';
  if (hook.active) return hook.kind === 'api' ? 'Connected' : 'Running';
  return hook.kind === 'api' ? 'Disconnected' : 'Stopped';
}

function HookCard({ hook, selected, onSelect }: { hook: HookConnection; selected: boolean; onSelect: () => void }) {
  const descriptor = descriptorFor(hook);
  const tone = !hook.installed ? 'available' : hook.active && hook.error === null ? 'good' : 'warning';
  return (
    <button class={`hook-card${selected ? ' is-selected' : ''}`} type="button" onClick={onSelect} aria-pressed={selected}>
      <span class="hook-card__icon" style={{ '--hook-color': descriptor.color }}><Icon name={descriptor.icon} size={22} /></span>
      <span class="hook-card__copy"><span>{descriptor.category}</span><strong>{descriptor.name}</strong><small>{descriptor.summary}</small></span>
      <span class={`hook-card__state hook-card__state--${tone}`}><i />{statusLabel(hook)}</span>
    </button>
  );
}

function HookDetails({ hook, canManage, busyAction, onAction }: {
  hook: HookConnection;
  canManage: boolean;
  busyAction: HookServiceAction | null;
  onAction: (action: HookServiceAction) => void;
}) {
  const descriptor = descriptorFor(hook);
  const href = panelHref(hook);
  const operationalActions = (['start', 'stop', 'restart'] as const).filter((action) => hook.actions.includes(action));
  return (
    <section class="hook-detail surface">
      <header class="hook-detail__head">
        <span class="hook-detail__logo" style={{ '--hook-color': descriptor.color }}><Icon name={descriptor.icon} size={26} /></span>
        <div><span>{descriptor.category}</span><h2>{descriptor.name}</h2><p>{descriptor.detail}</p></div>
        <span class={`state-label state-label--${hook.active && hook.error === null ? 'good' : 'idle'}`}>{statusLabel(hook)}</span>
      </header>
      {hook.error !== null && <div class="hook-warning" role="status"><Icon name="warning" size={15} /><span>{hook.error}</span></div>}
      {hook.installed ? (
        <>
          <dl class="hook-facts">
            <div><dt>Connection</dt><dd>{hook.kind === 'api' ? 'Protected API adapter' : hook.unit}</dd></div>
            <div><dt>Runtime</dt><dd>{hook.activeState.replaceAll('_', ' ')}</dd></div>
            <div><dt>After boot</dt><dd>{hook.enabledState.replaceAll('_', ' ')}</dd></div>
            {hook.instanceCount !== null && <div><dt>Imported servers</dt><dd>{hook.instanceCount}{hook.unverifiedInstanceCount ? ` · ${hook.unverifiedInstanceCount} unverified` : ''}</dd></div>}
          </dl>
          <div class="hook-actions">
            {operationalActions.map((action) => <button key={action} class={`button ${action === 'stop' ? 'button--danger-quiet' : action === 'restart' ? 'button--quiet' : 'button--primary'}`} type="button" disabled={!canManage || !hook.controllable || busyAction !== null || (action === 'start' && hook.active) || (action === 'stop' && !hook.active)} onClick={() => onAction(action)} aria-busy={busyAction === action}><Icon name={action === 'start' ? 'play' : action === 'stop' ? 'stop' : 'restart'} size={14} />{busyAction === action ? `${actionLabels[action]}ing…` : actionLabels[action]}</button>)}
            {href !== null && <a class="button button--quiet" href={href} target="_blank" rel="noopener noreferrer"><Icon name="external" size={14} />Open {descriptor.name}</a>}
          </div>
          {hook.kind === 'systemd' && <div class="hook-boot-control"><span><strong>Start after host boot</strong><small>Changes only <code>{hook.unit}</code>. It does not stop or start the service now.</small></span><button class="switch-button" role="switch" type="button" disabled={!canManage || !hook.controllable || busyAction !== null} aria-checked={hook.enabled} aria-busy={busyAction === 'enable' || busyAction === 'disable'} onClick={() => onAction(hook.enabled ? 'disable' : 'enable')}><i /><span>{busyAction === 'enable' || busyAction === 'disable' ? 'Saving…' : hook.enabled ? 'On' : 'Off'}</span></button></div>}
          {!hook.controllable && <small class="hook-boundary"><Icon name="info" size={13} />This adapter is read-only here. Use its own panel for settings Helix cannot verify safely.</small>}
        </>
      ) : (
        <div class="hook-setup">
          <div><span class="eyebrow">Guided setup</span><h3>Connect {descriptor.name}</h3><p>Helix has not found this service. These steps avoid guessing at a distribution, repository, or account.</p></div>
          <ol>{descriptor.setupSteps.map((step, index) => <li key={step}><span>{index + 1}</span><p>{step}</p></li>)}</ol>
          {descriptor.docs.length > 0 && <a class="button button--primary" href={descriptor.docs} target="_blank" rel="noopener noreferrer"><Icon name="external" size={14} />Open official setup</a>}
          <small><Icon name="info" size={13} />Helix does not run remote install scripts or invent credentials. Once the supported service exists, this card becomes operational automatically.</small>
        </div>
      )}
    </section>
  );
}

export interface HooksPageProps {
  csrfToken: string;
  canManage: boolean;
  onSessionExpired: () => void;
}

export function HooksPage({ csrfToken, canManage, onSessionExpired }: HooksPageProps) {
  const [inventory, setInventory] = useState<HookInventory | null>(null);
  const [selectedId, setSelectedId] = useState('plex');
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [busyAction, setBusyAction] = useState<HookServiceAction | null>(null);
  const [pendingAction, setPendingAction] = useState<HookServiceAction | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const refresh = useCallback(async (signal?: AbortSignal): Promise<void> => {
    setLoading(true);
    setError(null);
    try {
      const next = await getHookInventory(csrfToken, signal);
      setInventory(next);
      setSelectedId((current) => next.hooks.some((hook) => hook.id === current) ? current : next.hooks[0]?.id ?? '');
    } catch (reason) {
      if (reason instanceof ApiError && reason.status === 401) onSessionExpired();
      else if (signal?.aborted !== true) setError(describeError(reason));
    } finally {
      if (signal?.aborted !== true) setLoading(false);
    }
  }, [csrfToken, onSessionExpired]);

  useEffect(() => {
    const controller = new AbortController();
    void refresh(controller.signal);
    return () => controller.abort();
  }, [refresh]);

  const hooks = inventory?.hooks ?? [];
  const sorted = useMemo(() => [...hooks].sort((left, right) => Number(right.installed) - Number(left.installed) || descriptorFor(left).name.localeCompare(descriptorFor(right).name)), [hooks]);
  const selected = hooks.find((hook) => hook.id === selectedId) ?? sorted[0] ?? null;

  const runAction = async (): Promise<void> => {
    if (selected === null || pendingAction === null || busyAction !== null) return;
    const action = pendingAction;
    setPendingAction(null);
    setBusyAction(action);
    setError(null);
    setNotice(null);
    try {
      await manageHookService(selected.id, action, csrfToken);
      setNotice(`${descriptorFor(selected).name}: ${actionLabels[action]} completed and verified.`);
      await refresh();
    } catch (reason) {
      if (reason instanceof ApiError && reason.status === 401) onSessionExpired();
      else setError(describeError(reason));
    } finally {
      setBusyAction(null);
    }
  };

  return (
    <div class="page page--hooks">
      <PageHead title="Hooks" detail="Bring host services into Helix without pretending Helix owns them." />
      <div class="hooks-toolbar"><div><strong>{hooks.filter((hook) => hook.installed && hook.active).length}</strong><span>connected</span><i /><strong>{hooks.filter((hook) => !hook.installed).length}</strong><span>available</span><InfoTip text="Discovery is read-only. Control is exposed only for exact services configured in the root-owned broker, and every change is verified after systemd returns." /></div><button class="button button--quiet" type="button" disabled={loading} aria-busy={loading} onClick={() => void refresh()}><Icon name="refresh" size={14} />{loading ? 'Checking…' : 'Check connections'}</button></div>
      <InlineError message={error} />
      {notice !== null && <div class="hooks-notice" role="status"><Icon name="check" size={15} />{notice}</div>}
      {inventory === null && loading ? <div class="detail-loading" aria-busy="true"><Icon name="hooks" size={28} /><span>Discovering safe connections…</span></div> : <div class="hooks-layout"><aside class="hooks-list" aria-label="Available Hooks">{sorted.map((hook) => <HookCard key={hook.id} hook={hook} selected={selected?.id === hook.id} onSelect={() => { setSelectedId(hook.id); setNotice(null); }} />)}</aside>{selected !== null && <HookDetails hook={selected} canManage={canManage} busyAction={busyAction} onAction={(action) => setPendingAction(action)} />}</div>}
      {pendingAction !== null && selected !== null && <Dialog title={`${actionLabels[pendingAction]} ${descriptorFor(selected).name}?`} onClose={() => setPendingAction(null)}><p class="hook-confirm-copy">{pendingAction === 'stop' || pendingAction === 'restart' ? 'Active users or streams may be interrupted. Helix will issue the exact systemd action and verify the resulting state.' : 'Helix will change only the configured service and verify the result.'}</p><div class="hook-confirm-actions"><button class="button button--quiet" type="button" onClick={() => setPendingAction(null)}>Cancel</button><button class={`button ${pendingAction === 'stop' ? 'button--danger' : 'button--primary'}`} type="button" onClick={() => void runAction()}>{actionLabels[pendingAction]}</button></div></Dialog>}
    </div>
  );
}
