import { useEffect, useState } from 'preact/hooks';
import { InlineError, PageHead } from './dashboard-ui';
import { Icon } from './icons';
import type { TerminalPage as TerminalPageComponent, TerminalPageProps } from './terminal';

type TerminalComponent = typeof TerminalPageComponent;
let loadedTerminal: TerminalComponent | null = null;
let pendingTerminal: Promise<TerminalComponent> | null = null;

export function loadTerminalRoute(): Promise<TerminalComponent> {
  if (loadedTerminal !== null) return Promise.resolve(loadedTerminal);
  pendingTerminal ??= import('./terminal').then((module) => {
    loadedTerminal = module.TerminalPage;
    return loadedTerminal;
  }).catch((error: unknown) => {
    pendingTerminal = null;
    throw error;
  });
  return pendingTerminal;
}

export function preloadTerminalRoute(): void {
  void loadTerminalRoute().catch(() => undefined);
}

export function TerminalRoute(props: TerminalPageProps) {
  const [Page, setPage] = useState<TerminalComponent | null>(() => loadedTerminal);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Page === null && error === null
    ? loadTerminalRoute()
    : null;
  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => {
      if (mounted) setPage(() => component);
    }).catch(() => {
      if (mounted) setError('Terminal controls could not be loaded.');
    });
    return () => { mounted = false; };
  }, [request]);
  if (Page !== null) return <Page {...props} />;
  return <div class="page page--terminal"><PageHead title="Terminal" detail="Direct Linux shell access with a fresh password check." /><InlineError message={error} /><div class="detail-loading" aria-busy={error === null}><Icon name={error === null ? 'terminal' : 'warning'} size={28} /><span>{error === null ? 'Loading terminal…' : 'The rest of Helix is still available.'}</span>{error !== null && <button class="button button--primary" type="button" onClick={() => setError(null)}>Try again</button>}</div></div>;
}
