import { useEffect, useState } from 'preact/hooks';
import { InlineError, PageHead } from './dashboard-ui';
import { Icon } from './icons';
import type { OverviewPage as OverviewPageComponent, OverviewPageProps } from './overview';

type OverviewComponent = typeof OverviewPageComponent;
let loadedOverview: OverviewComponent | null = null;
let pendingOverview: Promise<OverviewComponent> | null = null;

export function loadOverviewRoute(): Promise<OverviewComponent> {
  if (loadedOverview !== null) return Promise.resolve(loadedOverview);
  pendingOverview ??= import('./overview').then((module) => {
    loadedOverview = module.OverviewPage;
    return loadedOverview;
  }).catch((error: unknown) => {
    pendingOverview = null;
    throw error;
  });
  return pendingOverview;
}

export function preloadOverviewRoute(): void {
  void loadOverviewRoute().catch(() => undefined);
}

export function OverviewRoute(props: OverviewPageProps) {
  const [Page, setPage] = useState<OverviewComponent | null>(() => loadedOverview);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Page === null && error === null
    ? loadOverviewRoute()
    : null;
  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => { if (mounted) setPage(() => component); }).catch(() => {
      if (mounted) setError('Overview could not be loaded.');
    });
    return () => { mounted = false; };
  }, [request]);
  if (Page !== null) return <Page {...props} />;
  return (
    <div class="page page--overview">
      <PageHead title="Overview" detail="The host, storage, and game workloads in one place." />
      <InlineError message={error} />
      <div class="detail-loading" aria-busy={error === null}>
        <Icon name={error === null ? 'activity' : 'warning'} size={28} />
        <span>{error === null ? 'Loading Overview…' : 'The rest of Helix is still available.'}</span>
        {error !== null && <button class="button button--primary" type="button" onClick={() => setError(null)}>Try again</button>}
      </div>
    </div>
  );
}
