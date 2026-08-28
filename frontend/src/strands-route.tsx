import { useEffect, useState } from 'preact/hooks';
import { InlineError } from './dashboard-ui';
import { Icon } from './icons';
import type { StrandsPage as StrandsPageComponent, StrandsPageProps } from './strands';

type StrandsComponent = typeof StrandsPageComponent;
let loadedStrands: StrandsComponent | null = null;
let pendingStrands: Promise<StrandsComponent> | null = null;

export function loadStrandsRoute(): Promise<StrandsComponent> {
  if (loadedStrands !== null) return Promise.resolve(loadedStrands);
  pendingStrands ??= import('./strands').then((module) => {
    loadedStrands = module.StrandsPage;
    return loadedStrands;
  }).catch((error: unknown) => {
    pendingStrands = null;
    throw error;
  });
  return pendingStrands;
}

export function preloadStrandsRoute(): void {
  void loadStrandsRoute().catch(() => undefined);
}

export function StrandsRoute(props: StrandsPageProps) {
  const [Page, setPage] = useState<StrandsComponent | null>(() => loadedStrands);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Page === null && error === null ? loadStrandsRoute() : null;
  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => { if (mounted) setPage(() => component); }).catch(() => { if (mounted) setError('Strands could not be loaded.'); });
    return () => { mounted = false; };
  }, [request]);
  if (Page !== null) return <Page {...props} />;
  return <div class="page page--strands"><InlineError message={error} /><div class="detail-loading" aria-busy={error === null}><Icon name={error === null ? 'strands' : 'warning'} size={28} /><span>{error === null ? 'Loading Strands…' : 'The rest of Helix is still available.'}</span>{error !== null && <button class="button button--primary" type="button" onClick={() => setError(null)}>Try again</button>}</div></div>;
}
