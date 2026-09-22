import { useEffect, useState } from 'preact/hooks';
import { InlineError, PageHead } from './dashboard-ui';
import { Icon } from './icons';
import type { MachinesPage as MachinesPageComponent, MachinesPageProps } from './machines';

type MachinesComponent = typeof MachinesPageComponent;
let loadedMachines: MachinesComponent | null = null;
let pendingMachines: Promise<MachinesComponent> | null = null;

export function loadMachinesRoute(): Promise<MachinesComponent> {
  if (loadedMachines !== null) return Promise.resolve(loadedMachines);
  pendingMachines ??= import('./machines').then((module) => {
    loadedMachines = module.MachinesPage;
    return loadedMachines;
  }).catch((error: unknown) => {
    pendingMachines = null;
    throw error;
  });
  return pendingMachines;
}

export function preloadMachinesRoute(): void {
  void loadMachinesRoute().catch(() => undefined);
}

export function MachinesRoute(props: MachinesPageProps) {
  const [Page, setPage] = useState<MachinesComponent | null>(() => loadedMachines);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Page === null && error === null
    ? loadMachinesRoute()
    : null;
  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => {
      if (mounted) setPage(() => component);
    }).catch(() => {
      if (mounted) setError('Machine controls could not be loaded.');
    });
    return () => { mounted = false; };
  }, [request]);
  if (Page !== null) return <Page {...props} />;
  return <div class="page"><PageHead title="Machines" detail="Rack machine registry and SSH terminals." /><InlineError message={error} /><div class="detail-loading" aria-busy={error === null}><Icon name={error === null ? 'servers' : 'warning'} size={28} /><span>{error === null ? 'Loading machines…' : 'The rest of Helix is still available.'}</span>{error !== null && <button class="button button--primary" type="button" onClick={() => setError(null)}>Try again</button>}</div></div>;
}
