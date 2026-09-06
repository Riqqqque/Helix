import { useEffect, useState } from 'preact/hooks';
import { InlineError } from './dashboard-ui';
import type { HomePage as HomePageComponent, HomePageProps } from './home';
import { Icon } from './icons';

type HomeComponent = typeof HomePageComponent;

let loadedHome: HomeComponent | null = null;
let pendingHome: Promise<HomeComponent> | null = null;

export function loadHomeRoute(): Promise<HomeComponent> {
  if (loadedHome !== null) return Promise.resolve(loadedHome);
  pendingHome ??= import('./home').then((module) => {
    loadedHome = module.HomePage;
    return loadedHome;
  }).catch((error: unknown) => {
    pendingHome = null;
    throw error;
  });
  return pendingHome;
}

export function preloadHomeRoute(): void {
  void loadHomeRoute().catch(() => undefined);
}

export function HomeRoute(props: HomePageProps) {
  const [Home, setHome] = useState<HomeComponent | null>(() => loadedHome);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Home === null && error === null
    ? loadHomeRoute()
    : null;

  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => {
      if (mounted) setHome(() => component);
    }).catch(() => {
      if (mounted) setError('Home could not be loaded.');
    });
    return () => {
      mounted = false;
    };
  }, [request]);

  if (Home !== null) return <Home {...props} />;
  return <div class="page page--home"><InlineError message={error} /><div class="detail-loading" aria-busy={error === null}><Icon name={error === null ? 'home' : 'warning'} size={26} /><span>{error === null ? 'Loading Home…' : 'The rest of Helix is still available.'}</span>{error !== null && <button class="button button--primary" type="button" onClick={() => setError(null)}>Try again</button>}</div></div>;
}
