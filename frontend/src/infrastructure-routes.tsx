import { useEffect, useState } from 'preact/hooks';
import { InlineError } from './dashboard-ui';
import { Icon } from './icons';
import type { HostUpdatesPanel } from './host-updates';
import type { NetworkOperationsPanel } from './network-panel';

type NetworkComponent = typeof NetworkOperationsPanel;
type UpdatesComponent = typeof HostUpdatesPanel;
export type NetworkOperationsRouteProps = Parameters<NetworkComponent>[0];
export type HostUpdatesRouteProps = Parameters<UpdatesComponent>[0];

let loadedNetwork: NetworkComponent | null = null;
let pendingNetwork: Promise<NetworkComponent> | null = null;
let loadedUpdates: UpdatesComponent | null = null;
let pendingUpdates: Promise<UpdatesComponent> | null = null;

export function loadNetworkOperationsRoute(): Promise<NetworkComponent> {
  if (loadedNetwork !== null) return Promise.resolve(loadedNetwork);
  pendingNetwork ??= import('./network-panel').then((module) => {
    loadedNetwork = module.NetworkOperationsPanel;
    return loadedNetwork;
  }).catch((error: unknown) => {
    pendingNetwork = null;
    throw error;
  });
  return pendingNetwork;
}

export function preloadNetworkOperationsRoute(): void {
  void loadNetworkOperationsRoute().catch(() => undefined);
}

export function loadHostUpdatesRoute(): Promise<UpdatesComponent> {
  if (loadedUpdates !== null) return Promise.resolve(loadedUpdates);
  pendingUpdates ??= import('./host-updates').then((module) => {
    loadedUpdates = module.HostUpdatesPanel;
    return loadedUpdates;
  }).catch((error: unknown) => {
    pendingUpdates = null;
    throw error;
  });
  return pendingUpdates;
}

export function preloadHostUpdatesRoute(): void {
  void loadHostUpdatesRoute().catch(() => undefined);
}

export function NetworkOperationsRoute(props: NetworkOperationsRouteProps) {
  const [Panel, setPanel] = useState<NetworkComponent | null>(() => loadedNetwork);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Panel === null && error === null ? loadNetworkOperationsRoute() : null;
  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => { if (mounted) setPanel(() => component); }).catch(() => { if (mounted) setError('Detailed network controls could not be loaded.'); });
    return () => { mounted = false; };
  }, [request]);
  if (Panel !== null) return <Panel {...props} />;
  return <section class="infrastructure-panel infrastructure-lazy" aria-busy={error === null}><InlineError message={error} /><div class="detail-loading"><Icon name={error === null ? 'network' : 'warning'} size={26} /><span>{error === null ? 'Loading network evidence…' : 'Basic interface data remains available above.'}</span>{error !== null && <button class="button button--primary" type="button" onClick={() => setError(null)}>Try again</button>}</div></section>;
}

export function HostUpdatesRoute(props: HostUpdatesRouteProps) {
  const [Panel, setPanel] = useState<UpdatesComponent | null>(() => loadedUpdates);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Panel === null && error === null ? loadHostUpdatesRoute() : null;
  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => { if (mounted) setPanel(() => component); }).catch(() => { if (mounted) setError('System updates could not be loaded.'); });
    return () => { mounted = false; };
  }, [request]);
  if (Panel !== null) return <Panel {...props} />;
  return <section class="infrastructure-panel infrastructure-lazy" aria-busy={error === null}><InlineError message={error} /><div class="detail-loading"><Icon name={error === null ? 'update' : 'warning'} size={26} /><span>{error === null ? 'Loading package inventory…' : 'Host services and processes remain available above.'}</span>{error !== null && <button class="button button--primary" type="button" onClick={() => setError(null)}>Try again</button>}</div></section>;
}
