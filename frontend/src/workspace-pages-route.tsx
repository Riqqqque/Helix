import { useEffect, useState } from 'preact/hooks';
import { InlineError } from './dashboard-ui';
import { Icon } from './icons';
import type { HostPage as HostPageComponent, NetworkPage as NetworkPageComponent, StoragePage as StoragePageComponent } from './workspace-pages';
import type { DashboardData } from './dashboard-model';

type StorageComponent = typeof StoragePageComponent;
type NetworkComponent = typeof NetworkPageComponent;
type HostComponent = typeof HostPageComponent;

let loadedStorage: StorageComponent | null = null;
let loadedNetwork: NetworkComponent | null = null;
let loadedHost: HostComponent | null = null;
let pendingWorkspace: Promise<void> | null = null;

function loadWorkspacePages(): Promise<void> {
  pendingWorkspace ??= import('./workspace-pages').then((module) => {
    loadedStorage = module.StoragePage;
    loadedNetwork = module.NetworkPage;
    loadedHost = module.HostPage;
  }).catch((error: unknown) => {
    pendingWorkspace = null;
    throw error;
  });
  return pendingWorkspace;
}

export function preloadWorkspacePages(): void {
  void loadWorkspacePages().catch(() => undefined);
}

function WorkspaceFallback({
  pageClass,
  icon,
  label,
  error,
  onRetry,
}: {
  pageClass: string;
  icon: 'storage' | 'network' | 'host';
  label: string;
  error: string | null;
  onRetry: () => void;
}) {
  return (
    <div class={`page ${pageClass}`}>
      <InlineError message={error} />
      <div class="detail-loading" aria-busy={error === null}>
        <Icon name={error === null ? icon : 'warning'} size={28} />
        <span>{error === null ? `Loading ${label}…` : 'The rest of Helix is still available.'}</span>
        {error !== null && <button class="button button--primary" type="button" onClick={onRetry}>Try again</button>}
      </div>
    </div>
  );
}

export function StoragePageRoute(props: { data: DashboardData; csrfToken: string; onSessionExpired: () => void }) {
  const [Page, setPage] = useState<StorageComponent | null>(() => loadedStorage);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Page === null && error === null ? loadWorkspacePages() : null;
  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then(() => {
      const next = loadedStorage;
      if (mounted && next !== null) setPage(() => next);
    }).catch(() => { if (mounted) setError('Storage could not be loaded.'); });
    return () => { mounted = false; };
  }, [request]);
  if (Page !== null) return <Page {...props} />;
  return <WorkspaceFallback pageClass="page--storage" icon="storage" label="Storage" error={error} onRetry={() => setError(null)} />;
}

export function NetworkPageRoute(props: { data: DashboardData; csrfToken: string; canManageFirewall: boolean; onSessionExpired: () => void }) {
  const [Page, setPage] = useState<NetworkComponent | null>(() => loadedNetwork);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Page === null && error === null ? loadWorkspacePages() : null;
  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then(() => {
      const next = loadedNetwork;
      if (mounted && next !== null) setPage(() => next);
    }).catch(() => { if (mounted) setError('Network could not be loaded.'); });
    return () => { mounted = false; };
  }, [request]);
  if (Page !== null) return <Page {...props} />;
  return <WorkspaceFallback pageClass="page--network" icon="network" label="Network" error={error} onRetry={() => setError(null)} />;
}

export function HostPageRoute(props: { data: DashboardData; csrfToken: string; canManageDocker: boolean; onSessionExpired: () => void }) {
  const [Page, setPage] = useState<HostComponent | null>(() => loadedHost);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Page === null && error === null ? loadWorkspacePages() : null;
  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then(() => {
      const next = loadedHost;
      if (mounted && next !== null) setPage(() => next);
    }).catch(() => { if (mounted) setError('Host could not be loaded.'); });
    return () => { mounted = false; };
  }, [request]);
  if (Page !== null) return <Page {...props} />;
  return <WorkspaceFallback pageClass="page--host" icon="host" label="Host" error={error} onRetry={() => setError(null)} />;
}
