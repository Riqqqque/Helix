import { useCallback, useEffect, useState } from 'preact/hooks';
import { ApiError } from './api';
import {
  getDockerInventory,
  portainerHref,
  runDockerContainerAction,
  type DockerContainer,
  type DockerContainerAction,
  type DockerInventory,
} from './docker-api';
import { formatBytes, formatPercent } from './format';
import { Icon } from './icons';
import { Dialog } from './modal';

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : 'Helix could not complete that Docker action.';
}

export function DockerInventoryPanel({
  csrfToken,
  canManage,
  compact = false,
  onSessionExpired,
}: {
  csrfToken: string;
  canManage: boolean;
  compact?: boolean;
  onSessionExpired: () => void;
}) {
  const [inventory, setInventory] = useState<DockerInventory | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState<{ container: DockerContainer; action: DockerContainerAction } | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async (signal?: AbortSignal): Promise<void> => {
    setLoading(true);
    try {
      const next = await getDockerInventory(csrfToken, signal);
      setInventory(next);
      setError(next.error);
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

  const run = async (): Promise<void> => {
    if (pending === null || busy) return;
    setBusy(true);
    setError(null);
    try {
      await runDockerContainerAction(pending.container.name, pending.action, pending.container.name, csrfToken);
      setPending(null);
      await refresh();
    } catch (reason) {
      if (reason instanceof ApiError && reason.status === 401) onSessionExpired();
      else setError(describeError(reason));
    } finally {
      setBusy(false);
    }
  };

  const containers = inventory?.containers ?? [];
  const href = typeof window === 'undefined'
    ? null
    : portainerHref(window.location.hostname, inventory?.portainer.panelPort ?? null, inventory?.portainer.panelScheme ?? null);
  return (
    <div class={`docker-panel${compact ? ' docker-panel--compact' : ''}`}>
      <div class="docker-panel__head">
        <div>
          <strong>{inventory?.dockerInstalled ? `${containers.filter((item) => item.running).length} running` : 'Docker unavailable'}</strong>
          <small>{inventory?.dockerInstalled ? `${containers.length} container${containers.length === 1 ? '' : 's'}` : 'Helix could not talk to the Docker engine on this host.'}</small>
        </div>
        <div class="docker-panel__actions">
          {href !== null && <a class="button button--quiet" href={href} target="_blank" rel="noopener noreferrer"><Icon name="external" size={14} />Open Portainer</a>}
          <button class="button button--quiet" type="button" disabled={loading} onClick={() => void refresh()}><Icon name="refresh" size={14} />{loading ? 'Checking…' : 'Refresh'}</button>
        </div>
      </div>
      {error !== null && <p class="docker-panel__error" role="status">{error}</p>}
      <div class="docker-list">
        {containers.slice(0, compact ? 8 : 64).map((container) => (
          <article class={`docker-row${container.running ? ' is-running' : ''}${container.protected ? ' is-protected' : ''}`} key={container.name}>
            <span class={`status-dot status-dot--${container.running ? 'good' : 'idle'}`} />
            <div>
              <strong>{container.name}</strong>
              <small>{container.image} · {container.status || container.state}</small>
            </div>
            <span>{container.cpuPercent === null ? '—' : formatPercent(container.cpuPercent)}</span>
            <span>{container.memoryUsedBytes === null ? '—' : formatBytes(container.memoryUsedBytes)}</span>
            {!compact && (
              <div class="docker-row__actions">
                <button class="button button--quiet" type="button" disabled={!canManage || container.protected || busy || container.running} onClick={() => setPending({ container, action: 'start' })}>Start</button>
                <button class="button button--quiet" type="button" disabled={!canManage || container.protected || busy || !container.running} onClick={() => setPending({ container, action: 'restart' })}>Restart</button>
                <button class="button button--danger-quiet" type="button" disabled={!canManage || container.protected || busy || !container.running} onClick={() => setPending({ container, action: 'stop' })}>Stop</button>
              </div>
            )}
            {container.protected && <em>Helix</em>}
          </article>
        ))}
        {containers.length === 0 && !loading && <div class="docker-empty">No containers reported.</div>}
      </div>
      {pending !== null && (
        <Dialog title={`${pending.action[0]?.toUpperCase()}${pending.action.slice(1)} ${pending.container.name}?`} onClose={() => setPending(null)}>
          <p class="docker-confirm">Helix will run a typed Docker {pending.action} for <code>{pending.container.name}</code> and then re-read container state. Game servers and media stacks can drop users. Helix dashboard and gateway containers stay protected.</p>
          <div class="dialog-actions">
            <button class="button button--quiet" type="button" disabled={busy} onClick={() => setPending(null)}>Cancel</button>
            <button class={`button ${pending.action === 'stop' ? 'button--danger' : 'button--primary'}`} type="button" disabled={busy} onClick={() => void run()}>{busy ? 'Working…' : `${pending.action[0]?.toUpperCase()}${pending.action.slice(1)}`}</button>
          </div>
        </Dialog>
      )}
    </div>
  );
}
