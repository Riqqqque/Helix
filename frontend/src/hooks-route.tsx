import { useEffect, useState } from 'preact/hooks';
import { InlineError } from './dashboard-ui';
import type { HooksPage as HooksPageComponent, HooksPageProps } from './hooks';
import { Icon } from './icons';

type HooksComponent = typeof HooksPageComponent;
let loadedHooks: HooksComponent | null = null;
let pendingHooks: Promise<HooksComponent> | null = null;

export function loadHooksRoute(): Promise<HooksComponent> {
  if (loadedHooks !== null) return Promise.resolve(loadedHooks);
  pendingHooks ??= import('./hooks').then((module) => {
    loadedHooks = module.HooksPage;
    return loadedHooks;
  }).catch((error: unknown) => {
    pendingHooks = null;
    throw error;
  });
  return pendingHooks;
}

export function preloadHooksRoute(): void {
  void loadHooksRoute().catch(() => undefined);
}

export function HooksRoute(props: HooksPageProps) {
  const [Page, setPage] = useState<HooksComponent | null>(() => loadedHooks);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Page === null && error === null ? loadHooksRoute() : null;
  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => { if (mounted) setPage(() => component); }).catch(() => { if (mounted) setError('Hooks could not be loaded.'); });
    return () => { mounted = false; };
  }, [request]);
  if (Page !== null) return <Page {...props} />;
  return <div class="page page--hooks"><InlineError message={error} /><div class="detail-loading" aria-busy={error === null}><Icon name={error === null ? 'hooks' : 'warning'} size={28} /><span>{error === null ? 'Loading Hooks…' : 'The rest of Helix is still available.'}</span>{error !== null && <button class="button button--primary" type="button" onClick={() => setError(null)}>Try again</button>}</div></div>;
}
