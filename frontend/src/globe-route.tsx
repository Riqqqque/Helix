import { useEffect, useState } from 'preact/hooks';
import { InlineError } from './dashboard-ui';
import { Icon } from './icons';
import type { GlobePage as GlobePageComponent } from './globe';

type GlobeComponent = typeof GlobePageComponent;
let loadedGlobe: GlobeComponent | null = null;
let pendingGlobe: Promise<GlobeComponent> | null = null;

export function loadGlobeRoute(): Promise<GlobeComponent> {
  if (loadedGlobe !== null) return Promise.resolve(loadedGlobe);
  pendingGlobe ??= import('./globe').then((module) => {
    loadedGlobe = module.GlobePage;
    return loadedGlobe;
  }).catch((error: unknown) => {
    pendingGlobe = null;
    throw error;
  });
  return pendingGlobe;
}

export function preloadGlobeRoute(): void {
  void loadGlobeRoute().catch(() => undefined);
}

export function GlobeRoute(props: { csrfToken: string; onSessionExpired: () => void }) {
  const [Page, setPage] = useState<GlobeComponent | null>(() => loadedGlobe);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Page === null && error === null ? loadGlobeRoute() : null;
  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => { if (mounted) setPage(() => component); }).catch(() => { if (mounted) setError('Globe could not be loaded.'); });
    return () => { mounted = false; };
  }, [request]);
  if (Page !== null) return <Page {...props} />;
  return <div class="page page--globe"><InlineError message={error} /><div class="detail-loading" aria-busy={error === null}><Icon name={error === null ? 'globe' : 'warning'} size={28} /><span>{error === null ? 'Loading Globe…' : 'The rest of Helix is still available.'}</span>{error !== null && <button class="button button--primary" type="button" onClick={() => setError(null)}>Try again</button>}</div></div>;
}
