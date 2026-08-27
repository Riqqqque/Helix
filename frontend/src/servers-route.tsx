import { useEffect, useState } from 'preact/hooks';
import type { DashboardData } from './dashboard-model';
import { InlineError, PageHead } from './dashboard-ui';
import { Icon } from './icons';
import type { ServersPage } from './servers';

export interface ServersRouteProps {
  data: DashboardData;
  csrfToken: string;
  canManageServers: boolean;
  canManageBackups: boolean;
  onSessionExpired: () => void;
}

type ServersPageComponent = typeof ServersPage;

let loadedPage: ServersPageComponent | null = null;
let pendingPage: Promise<ServersPageComponent> | null = null;

export function loadServersRoute(): Promise<ServersPageComponent> {
  if (loadedPage !== null) return Promise.resolve(loadedPage);
  pendingPage ??= import('./servers').then((module) => {
    loadedPage = module.ServersPage;
    return loadedPage;
  }).catch((error: unknown) => {
    pendingPage = null;
    throw error;
  });
  return pendingPage;
}

export function preloadServersRoute(): void {
  void loadServersRoute().catch(() => undefined);
}

export function ServersRoute(props: ServersRouteProps) {
  const [Page, setPage] = useState<ServersPageComponent | null>(() => loadedPage);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Page === null && error === null
    ? loadServersRoute()
    : null;

  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => {
      if (mounted) setPage(() => component);
    }).catch(() => {
      if (mounted) setError('Server controls could not be loaded.');
    });
    return () => { mounted = false; };
  }, [request]);

  if (Page !== null) return <Page {...props} />;
  return (
    <div class="page page--servers" aria-busy={error === null}>
      <PageHead title="Servers" detail="Helix’s own game-server manager." />
      <InlineError message={error} />
      <div class="detail-loading" role="status" aria-live="polite">
        <Icon name={error === null ? 'servers' : 'warning'} size={28} />
        <span>{error === null ? 'Loading server controls…' : 'The rest of the dashboard is still available.'}</span>
        {error !== null && <button class="button button--primary" type="button" onClick={() => setError(null)}>Try again</button>}
      </div>
    </div>
  );
}
