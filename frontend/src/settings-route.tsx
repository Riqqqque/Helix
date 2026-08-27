import { useEffect, useState } from 'preact/hooks';
import { InlineError, PageHead } from './dashboard-ui';
import { Icon } from './icons';
import type { DashboardSettingsPage } from './dashboard-settings';

type SettingsPageComponent = typeof DashboardSettingsPage;
export type SettingsRouteProps = Parameters<SettingsPageComponent>[0];

let loadedPage: SettingsPageComponent | null = null;
let pendingPage: Promise<SettingsPageComponent> | null = null;

export function loadSettingsRoute(): Promise<SettingsPageComponent> {
  if (loadedPage !== null) return Promise.resolve(loadedPage);
  pendingPage ??= import('./dashboard-settings').then((module) => {
    loadedPage = module.DashboardSettingsPage;
    return loadedPage;
  }).catch((error: unknown) => {
    pendingPage = null;
    throw error;
  });
  return pendingPage;
}

export function preloadSettingsRoute(): void {
  void loadSettingsRoute().catch(() => undefined);
}

export function SettingsRoute(props: SettingsRouteProps) {
  const [Page, setPage] = useState<SettingsPageComponent | null>(() => loadedPage);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Page === null && error === null ? loadSettingsRoute() : null;

  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => {
      if (mounted) setPage(() => component);
    }).catch(() => {
      if (mounted) setError('Settings could not be loaded.');
    });
    return () => { mounted = false; };
  }, [request]);

  if (Page !== null) return <Page {...props} />;
  return <div class="page page--dashboard-settings" aria-busy={error === null}><PageHead title="Settings" detail="Dashboard behavior, host integration, and owner security." /><InlineError message={error} /><div class="detail-loading" role="status" aria-live="polite"><Icon name={error === null ? 'settings' : 'warning'} size={28} /><span>{error === null ? 'Loading settings…' : 'The rest of the dashboard is still available.'}</span>{error !== null && <button class="button button--primary" type="button" onClick={() => setError(null)}>Try again</button>}</div></div>;
}
