import { useEffect, useState } from 'preact/hooks';
import { InlineError } from './dashboard-ui';
import { Icon } from './icons';
import type { MarketplaceServerContext } from './marketplace-api';
import type { MarketplacePanel } from './marketplace';

export interface MarketplaceRouteProps {
  server: MarketplaceServerContext;
  csrfToken: string;
  canManageServers: boolean;
  onSessionExpired: () => void;
  onInstalled: () => Promise<void>;
}

type MarketplacePanelComponent = typeof MarketplacePanel;

let loadedPanel: MarketplacePanelComponent | null = null;
let pendingPanel: Promise<MarketplacePanelComponent> | null = null;

export function loadMarketplaceRoute(): Promise<MarketplacePanelComponent> {
  if (loadedPanel !== null) return Promise.resolve(loadedPanel);
  pendingPanel ??= import('./marketplace').then((module) => {
    loadedPanel = module.MarketplacePanel;
    return loadedPanel;
  }).catch((error: unknown) => {
    pendingPanel = null;
    throw error;
  });
  return pendingPanel;
}

export function preloadMarketplaceRoute(): void {
  void loadMarketplaceRoute().catch(() => undefined);
}

export function MarketplaceRoute(props: MarketplaceRouteProps) {
  const [Panel, setPanel] = useState<MarketplacePanelComponent | null>(() => loadedPanel);
  const [error, setError] = useState<string | null>(null);
  const [loadGeneration, setLoadGeneration] = useState(0);

  useEffect(() => {
    if (Panel !== null) return;
    let mounted = true;
    let attempt = 0;
    let timer: number | null = null;
    const tryLoad = (): void => {
      void loadMarketplaceRoute()
        .then((component) => {
          if (!mounted) return;
          setError(null);
          setPanel(() => component);
        })
        .catch(() => {
          if (!mounted) return;
          attempt += 1;
          if (attempt < 3) {
            timer = window.setTimeout(tryLoad, 400 * attempt);
            return;
          }
          setError('The marketplace could not be loaded.');
        });
    };
    tryLoad();
    return () => {
      mounted = false;
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [Panel, loadGeneration]);

  if (Panel !== null) return <Panel {...props} />;
  return (
    <section class="server-tool marketplace-loading" aria-busy={error === null}>
      <InlineError message={error} />
      <div class="detail-loading" role="status" aria-live="polite">
        <Icon name={error === null ? 'search' : 'warning'} size={28} />
        <span>{error === null ? 'Opening the marketplace…' : 'Your other server tools are still available.'}</span>
        {error !== null && (
          <button class="button button--primary" type="button" onClick={() => { setError(null); setLoadGeneration((value) => value + 1); }}>
            Try again
          </button>
        )}
      </div>
    </section>
  );
}
