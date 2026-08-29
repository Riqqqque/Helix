import { useEffect, useState } from 'preact/hooks';
import { InlineError, PageHead } from './dashboard-ui';
import { Icon } from './icons';
import type { SecurityPage as SecurityPageComponent, SecurityPageProps } from './security';

type SecurityComponent = typeof SecurityPageComponent;
let loadedSecurity: SecurityComponent | null = null;
let pendingSecurity: Promise<SecurityComponent> | null = null;

export function loadSecurityRoute(): Promise<SecurityComponent> {
  if (loadedSecurity !== null) return Promise.resolve(loadedSecurity);
  pendingSecurity ??= import('./security').then((module) => {
    loadedSecurity = module.SecurityPage;
    return loadedSecurity;
  }).catch((error: unknown) => {
    pendingSecurity = null;
    throw error;
  });
  return pendingSecurity;
}

export function preloadSecurityRoute(csrfToken?: string): void {
  void loadSecurityRoute().catch(() => undefined);
  if (csrfToken !== undefined) {
    void import('./security-api').then((module) => {
      module.prefetchSecurityInventory(csrfToken);
    }).catch(() => undefined);
  }
}

export function SecurityRoute(props: SecurityPageProps) {
  const [Page, setPage] = useState<SecurityComponent | null>(() => loadedSecurity);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Page === null && error === null ? loadSecurityRoute() : null;
  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => { if (mounted) setPage(() => component); }).catch(() => {
      if (mounted) setError('Security could not be loaded.');
    });
    return () => { mounted = false; };
  }, [request]);
  if (Page !== null) return <Page {...props} />;
  return (
    <div class="page page--security">
      <PageHead title="Security" detail="Exact host and Helix controls, with the reason each one exists." />
      <InlineError message={error} />
      <div class="detail-loading" aria-busy={error === null}>
        <Icon name={error === null ? 'advanced' : 'warning'} size={28} />
        <span>{error === null ? 'Loading Security…' : 'The rest of Helix is still available.'}</span>
        {error !== null && <button class="button button--primary" type="button" onClick={() => setError(null)}>Try again</button>}
      </div>
    </div>
  );
}
