import { useCallback, useEffect, useMemo, useState } from 'preact/hooks';
import { ApiError } from './api';
import { InlineError, PageHead, ProgressBar } from './dashboard-ui';
import {
  getHookInstallJob,
  getHookInstallPlan,
  getHookInventory,
  installHook,
  manageHookService,
  type HookConnection,
  type HookInstallJob,
  type HookInstallPlan,
  type HookInventory,
  type HookServiceAction,
} from './hooks-api';
import { formatBytes, formatPercent } from './format';
import { Icon, type IconName } from './icons';
import { InfoTip } from './info-tip';
import { Dialog } from './modal';
import { DockerInventoryPanel } from './docker-panel';
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
    detail: 'On supported Debian and Ubuntu hosts, Helix can add Tailscale’s official signed repository, install the package, and verify tailscaled in one job. Tailnet approval stays with you because Helix never invents or stores account credentials.',
    icon: 'network',
    color: '#b9b9b9',
    docs: 'https://tailscale.com/docs/install/linux',
    setupSteps: ['Use the official Linux instructions for this distribution.', 'Run tailscale up and approve this host in the intended tailnet.', 'Return here and refresh; Helix will show service and boot state.'],
  },
  pterodactyl: {
    name: 'Pterodactyl Wings',
    category: 'Game hosting',
    summary: 'Node-agent health and lifecycle control',
    detail: 'Helix checks the Linux, Docker, architecture, and systemd prerequisites, then walks through the panel-owned node configuration. Wings cannot be honestly reduced to one click because its config and credentials must come from your Pterodactyl Panel.',
    icon: 'servers',
    color: '#7c9cff',
    docs: 'https://pterodactyl.io/wings/1.0/installing.html',
    setupSteps: ['Install a supported Pterodactyl panel and Wings node.', 'Complete the node configuration and verify Wings independently.', 'Refresh Hooks to pick up wings.service.'],
  },
  jellyfin: {
    name: 'Jellyfin',
    category: 'Media',
    summary: 'Media server service control and quick access',
    detail: 'On supported Debian and Ubuntu hosts, Helix can add Jellyfin’s official signed repository, install its packages, and verify the service in one job. Library and owner setup then continue in Jellyfin Web.',
    icon: 'play',
    color: '#a56dff',
    docs: 'https://jellyfin.org/docs/general/installation/linux/',
    setupSteps: ['Install Jellyfin using its supported package instructions.', 'Complete the first-run setup in Jellyfin Web.', 'Refresh Hooks to detect jellyfin.service.'],
  },
  docker: {
    name: 'Docker',
    category: 'Containers',
    summary: 'Every container on this host, plus Portainer when it is present',
    detail: 'Helix reads Docker directly. You can see CPU, memory, and running state for all containers, start or stop them with the exact name, and open Portainer if that container is published. Helix dashboard and gateway containers stay protected.',
    icon: 'host',
    color: '#2496ed',
    docs: 'https://docs.docker.com/',
    setupSteps: ['Install Docker Engine on this Linux host.', 'Refresh Hooks; Helix lists every container Docker reports.', 'If Portainer is running, Open Portainer uses its published port on this LAN address.'],
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
  if (hook.id === 'docker' && hook.panelPort !== null) return `http://${hostname}:${hook.panelPort}/`;
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
      <span class="hook-card__copy">
        <span>{descriptor.category}</span>
        <strong>{descriptor.name}</strong>
        <small>{descriptor.summary}</small>
        {(hook.memoryUsedBytes !== null || hook.cpuPercent !== null) && (
          <em class="hook-card__resources">
            {hook.memoryUsedBytes !== null ? formatBytes(hook.memoryUsedBytes) : 'Memory unknown'}
            {hook.cpuPercent !== null ? ` · ${formatPercent(hook.cpuPercent)} CPU` : ''}
          </em>
        )}
      </span>
      <span class={`hook-card__state hook-card__state--${tone}`}><i />{statusLabel(hook)}</span>
    </button>
  );
}

function HookDetails({ hook, canManage, busyAction, plan, planLoading, planError, csrfToken, onSessionExpired, onAction, onInstall, onRetryPlan }: {
  hook: HookConnection;
  canManage: boolean;
  busyAction: HookServiceAction | null;
  plan: HookInstallPlan | null;
  planLoading: boolean;
  planError: string | null;
  csrfToken: string;
  onSessionExpired: () => void;
  onAction: (action: HookServiceAction) => void;
  onInstall: () => void;
  onRetryPlan: () => void;
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
            <div><dt>Connection</dt><dd>{hook.kind === 'api' ? 'Protected API adapter' : hook.kind === 'docker' ? 'Docker Engine' : hook.unit}</dd></div>
            <div><dt>Runtime</dt><dd>{hook.activeState.replaceAll('_', ' ')}</dd></div>
            <div><dt>After boot</dt><dd>{hook.enabledState.replaceAll('_', ' ')}</dd></div>
            {hook.instanceCount !== null && <div><dt>Imported servers</dt><dd>{hook.instanceCount}{hook.unverifiedInstanceCount ? ` · ${hook.unverifiedInstanceCount} unverified` : ''}</dd></div>}
            {(hook.memoryUsedBytes !== null || hook.cpuPercent !== null) && <div><dt>Host resources</dt><dd>{[hook.memoryUsedBytes === null ? null : formatBytes(hook.memoryUsedBytes), hook.cpuPercent === null ? null : `${formatPercent(hook.cpuPercent)} CPU`].filter(Boolean).join(' · ')}</dd></div>}
          </dl>
          <div class="hook-actions">
            {operationalActions.map((action) => <button key={action} class={`button ${action === 'stop' ? 'button--danger-quiet' : action === 'restart' ? 'button--quiet' : 'button--primary'}`} type="button" disabled={!canManage || !hook.controllable || busyAction !== null || (action === 'start' && hook.active) || (action === 'stop' && !hook.active)} onClick={() => onAction(action)} aria-busy={busyAction === action}><Icon name={action === 'start' ? 'play' : action === 'stop' ? 'stop' : 'restart'} size={14} />{busyAction === action ? `${actionLabels[action]}ing…` : actionLabels[action]}</button>)}
            {href !== null && <a class="button button--quiet" href={href} target="_blank" rel="noopener noreferrer"><Icon name="external" size={14} />Open {descriptor.name}</a>}
          </div>
          {hook.kind === 'systemd' && <div class="hook-boot-control"><span><strong>Start after host boot</strong><small>Changes only <code>{hook.unit}</code>. It does not stop or start the service now.</small></span><button class="switch-button" role="switch" type="button" disabled={!canManage || !hook.controllable || busyAction !== null} aria-checked={busyAction === 'enable' ? true : busyAction === 'disable' ? false : hook.enabled} aria-busy={busyAction === 'enable' || busyAction === 'disable'} onClick={() => onAction(hook.enabled ? 'disable' : 'enable')}><i /><span>{busyAction === 'enable' || busyAction === 'disable' ? 'Saving…' : hook.enabled ? 'On' : 'Off'}</span></button></div>}
          {!hook.controllable && <small class="hook-boundary"><Icon name="info" size={13} />This adapter is read-only here. Use its own panel for settings Helix cannot verify safely.</small>}
          {hook.id === 'docker' && <DockerInventoryPanel csrfToken={csrfToken} canManage={canManage} onSessionExpired={onSessionExpired} />}
        </>
      ) : (
        <div class="hook-setup">
          <div><span class="eyebrow">{plan?.mode === 'one_click' ? 'ONE-CLICK INSTALL' : 'GUIDED CONNECTION'}</span><h3>Connect {descriptor.name}</h3><p>{plan?.mode === 'one_click' ? 'Helix can install this from the publisher’s signed APT repository, then verify the exact systemd service.' : 'Helix checks this host first and shows only the steps that still need an owner decision.'}</p></div>
          {planLoading && <div class="hook-plan-loading" role="status"><Icon name="refresh" class="is-spinning" /><span>Checking this host…</span></div>}
          <InlineError message={planError} />
          {planError !== null && <button class="button button--quiet" type="button" onClick={onRetryPlan}><Icon name="refresh" size={14} />Check again</button>}
          {plan !== null && <>
            {plan.platform !== null && <div class="hook-platform"><Icon name="host" size={15} /><span><strong>{plan.platform.name}</strong><small>{plan.platform.codename} · {plan.platform.architecture}</small></span></div>}
            <div class="hook-checks">{plan.checks.map((check) => <div class={`hook-check hook-check--${check.status}`} key={check.id}><Icon name={check.status === 'pass' ? 'check' : 'warning'} size={14} /><span><strong>{check.label}</strong><small>{check.detail}</small></span></div>)}</div>
            {plan.blockers.length > 0 && <div class="hook-plan-blockers">{plan.blockers.map((blocker) => <p key={blocker}><Icon name="warning" size={13} />{blocker}</p>)}</div>}
            <section class="hook-plan-columns">
              <div><span class="eyebrow">HELIX WILL</span><ul>{plan.changes.map((change) => <li key={change}><Icon name="check" size={13} />{change}</li>)}</ul></div>
              <div><span class="eyebrow">YOU FINISH</span><ul>{plan.nextSteps.map((step) => <li key={step}><Icon name="chevron" size={13} />{step}</li>)}</ul></div>
            </section>
            <div class="hook-plan-actions">
              {plan.installAvailable && <button class="button button--primary" type="button" disabled={!canManage} onClick={onInstall}><Icon name="plus" size={14} />Install {descriptor.name}</button>}
              {plan.mode === 'guided' && <a class="button button--primary" href="#terminal"><Icon name="terminal" size={14} />Continue in Terminal</a>}
              <a class="button button--quiet" href={plan.officialDocs} target="_blank" rel="noopener noreferrer"><Icon name="external" size={14} />Official instructions</a>
            </div>
          </>}
          {plan === null && !planLoading && planError === null && <ol>{descriptor.setupSteps.map((step, index) => <li key={step}><span>{index + 1}</span><p>{step}</p></li>)}</ol>}
          <small><Icon name="info" size={13} />Installers are exact-ID allowlisted. Helix never accepts arbitrary package names, repositories, commands, or remote scripts from the browser.</small>
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
  const [plan, setPlan] = useState<HookInstallPlan | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [planLoading, setPlanLoading] = useState(false);
  const [planRevision, setPlanRevision] = useState(0);
  const [installOpen, setInstallOpen] = useState(false);
  const [installJob, setInstallJob] = useState<HookInstallJob | null>(null);
  const [installDispatching, setInstallDispatching] = useState(false);
  const [installConfirmed, setInstallConfirmed] = useState(false);

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

  useEffect(() => {
    if (selected === null || selected.installed || !['tailscale', 'jellyfin', 'pterodactyl'].includes(selected.id)) {
      setPlan(null);
      setPlanError(null);
      setPlanLoading(false);
      return;
    }
    const controller = new AbortController();
    setPlanLoading(true);
    setPlanError(null);
    void getHookInstallPlan(selected.id, csrfToken, controller.signal)
      .then((next) => setPlan(next))
      .catch((reason: unknown) => {
        if (controller.signal.aborted) return;
        if (reason instanceof ApiError && reason.status === 401) onSessionExpired();
        else {
          setPlan(null);
          setPlanError(describeError(reason));
        }
      })
      .finally(() => { if (!controller.signal.aborted) setPlanLoading(false); });
    return () => controller.abort();
  }, [csrfToken, onSessionExpired, planRevision, selected?.id, selected?.installed]);

  const runAction = async (): Promise<void> => {
    if (selected === null || pendingAction === null || busyAction !== null) return;
    const action = pendingAction;
    setPendingAction(null);
    setBusyAction(action);
    setError(null);
    setNotice(null);
    try {
      const result = await manageHookService(selected.id, action, csrfToken);
      setInventory((current) => current === null ? current : {
        ...current,
        hooks: current.hooks.map((hook) => hook.id !== result.hookId ? hook : {
          ...hook,
          active: result.active,
          activeState: result.activeState,
          enabled: result.enabled,
          enabledState: result.enabledState,
        }),
      });
      setNotice(`${descriptorFor(selected).name}: ${actionLabels[action]} completed and verified.`);
      void refresh();
    } catch (reason) {
      if (reason instanceof ApiError && reason.status === 401) onSessionExpired();
      else setError(describeError(reason));
    } finally {
      setBusyAction(null);
    }
  };

  const startInstall = async (): Promise<void> => {
    if (selected === null || !installConfirmed || installDispatching) return;
    setInstallDispatching(true);
    setError(null);
    try {
      const dispatch = await installHook(selected.id, csrfToken);
      setInstallJob({
        id: dispatch.jobId,
        kind: 'hook_install',
        status: 'queued',
        stage: dispatch.reused ? 'Joining the active installation' : 'Queued',
        progressPercent: 0,
        result: null,
        error: null,
      });
    } catch (reason) {
      if (reason instanceof ApiError && reason.status === 401) onSessionExpired();
      else setError(describeError(reason));
    } finally {
      setInstallDispatching(false);
    }
  };

  useEffect(() => {
    if (installJob === null || installJob.status === 'complete' || installJob.status === 'failed') return;
    const controller = new AbortController();
    let timer: number | null = null;
    const poll = async (): Promise<void> => {
      try {
        const next = await getHookInstallJob(installJob.id, csrfToken, controller.signal);
        setInstallJob(next);
        if (next.status === 'complete') {
          setNotice('Hook installation completed and the system service was verified.');
          void refresh();
        } else if (next.status !== 'failed') {
          timer = window.setTimeout(() => void poll(), 1_500);
        }
      } catch (reason) {
        if (controller.signal.aborted) return;
        if (reason instanceof ApiError && reason.status === 401) onSessionExpired();
        else setError(describeError(reason));
      }
    };
    timer = window.setTimeout(() => void poll(), 500);
    return () => {
      controller.abort();
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [csrfToken, installJob?.id, installJob?.status, onSessionExpired, refresh]);

  return (
    <div class="page page--hooks">
      <PageHead title="Hooks" detail="Bring host services into Helix without pretending Helix owns them." />
      <div class="hooks-toolbar"><div><strong>{hooks.filter((hook) => hook.installed && hook.active).length}</strong><span>connected</span><i /><strong>{hooks.filter((hook) => !hook.installed).length}</strong><span>available</span><InfoTip text="Discovery is read-only. Control is exposed only for exact services configured in the root-owned broker, and every change is verified after systemd returns." /></div><button class="button button--quiet" type="button" disabled={loading} aria-busy={loading} onClick={() => void refresh()}><Icon name="refresh" size={14} />{loading ? 'Checking…' : 'Check connections'}</button></div>
      <InlineError message={error} />
      {notice !== null && <div class="hooks-notice" role="status"><Icon name="check" size={15} />{notice}</div>}
      {inventory === null && loading ? <div class="detail-loading" aria-busy="true"><Icon name="hooks" size={28} /><span>Discovering safe connections…</span></div> : <div class="hooks-layout"><aside class="hooks-list" aria-label="Available Hooks">{sorted.map((hook) => <HookCard key={hook.id} hook={hook} selected={selected?.id === hook.id} onSelect={() => { setSelectedId(hook.id); setNotice(null); }} />)}</aside>{selected !== null && <HookDetails hook={selected} canManage={canManage} busyAction={busyAction} plan={plan} planLoading={planLoading} planError={planError} csrfToken={csrfToken} onSessionExpired={onSessionExpired} onAction={(action) => setPendingAction(action)} onInstall={() => { setInstallConfirmed(false); setInstallJob(null); setInstallOpen(true); }} onRetryPlan={() => setPlanRevision((value) => value + 1)} />}</div>}
      {pendingAction !== null && selected !== null && <Dialog title={`${actionLabels[pendingAction]} ${descriptorFor(selected).name}?`} onClose={() => setPendingAction(null)}><p class="hook-confirm-copy">{pendingAction === 'stop' || pendingAction === 'restart' ? 'Active users or streams may be interrupted. Helix will issue the exact systemd action and verify the resulting state.' : 'Helix will change only the configured service and verify the result.'}</p><div class="hook-confirm-actions"><button class="button button--quiet" type="button" onClick={() => setPendingAction(null)}>Cancel</button><button class={`button ${pendingAction === 'stop' ? 'button--danger' : 'button--primary'}`} type="button" onClick={() => void runAction()}>{actionLabels[pendingAction]}</button></div></Dialog>}
      {installOpen && selected !== null && <Dialog title={installJob === null ? `Install ${descriptorFor(selected).name}?` : installJob.status === 'complete' ? 'Installation complete' : installJob.status === 'failed' ? 'Installation needs attention' : `Installing ${descriptorFor(selected).name}`} onClose={() => { if (installJob === null || ['complete', 'failed'].includes(installJob.status)) setInstallOpen(false); }} wide>
        <InlineError message={error} />
        {installJob === null ? <>
          <div class="hook-install-review"><Icon name={descriptorFor(selected).icon} size={25} /><span><strong>Publisher repository, exact package, exact service</strong><p>Helix will add only the official signed repository shown in the preflight, install the allowlisted package, enable its exact systemd unit, and verify that it is active. It will not create an external account or reboot Linux.</p></span></div>
          <label class="check-row"><input type="checkbox" checked={installConfirmed} onChange={(event) => setInstallConfirmed(event.currentTarget.checked)} /><span><strong>Install {descriptorFor(selected).name} on this Linux host</strong><small>I understand this adds a publisher APT repository and packages to the host.</small></span></label>
          <div class="dialog-actions"><button class="button button--quiet" type="button" onClick={() => setInstallOpen(false)}>Cancel</button><button class="button button--primary" type="button" disabled={!installConfirmed || installDispatching} onClick={() => void startInstall()}>{installDispatching ? 'Queuing…' : 'Install and verify'}</button></div>
        </> : <>
          <div class={`hook-install-progress hook-install-progress--${installJob.status}`} role="status"><Icon name={installJob.status === 'complete' ? 'check' : installJob.status === 'failed' ? 'warning' : 'update'} size={26} class={installJob.status === 'running' ? 'is-spinning' : undefined} /><strong>{installJob.stage}</strong><ProgressBar value={Math.max(installJob.progressPercent, installJob.status === 'running' ? 8 : 0)} /><span>{installJob.status === 'complete' ? 'The package and exact systemd service were verified.' : installJob.status === 'failed' ? installJob.error ?? 'The installer stopped before verification.' : 'This continues on the host if you leave the page.'}</span></div>
          <div class="dialog-actions"><button class="button button--primary" type="button" disabled={!['complete', 'failed'].includes(installJob.status)} onClick={() => setInstallOpen(false)}>Close</button></div>
        </>}
      </Dialog>}
    </div>
  );
}
