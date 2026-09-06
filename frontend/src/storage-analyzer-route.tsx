import { useEffect, useState } from 'preact/hooks';
import { InlineError } from './dashboard-ui';
import { Icon } from './icons';
import { useModalFocus } from './modal-focus';
import type { StorageAnalyzer as StorageAnalyzerComponent, StorageAnalyzerProps } from './storage-analyzer';

type AnalyzerComponent = typeof StorageAnalyzerComponent;

let loadedAnalyzer: AnalyzerComponent | null = null;
let pendingAnalyzer: Promise<AnalyzerComponent> | null = null;

export function loadStorageAnalyzer(): Promise<AnalyzerComponent> {
  if (loadedAnalyzer !== null) return Promise.resolve(loadedAnalyzer);
  pendingAnalyzer ??= import('./storage-analyzer').then((module) => {
    loadedAnalyzer = module.StorageAnalyzer;
    return loadedAnalyzer;
  }).catch((error: unknown) => {
    pendingAnalyzer = null;
    throw error;
  });
  return pendingAnalyzer;
}

export function preloadStorageAnalyzer(): void {
  void loadStorageAnalyzer().catch(() => undefined);
}

export function StorageAnalyzerRoute(props: StorageAnalyzerProps) {
  const [Analyzer, setAnalyzer] = useState<AnalyzerComponent | null>(() => loadedAnalyzer);
  const [error, setError] = useState<string | null>(null);
  const modalRef = useModalFocus(props.onClose);
  const request = typeof document !== 'undefined' && Analyzer === null && error === null
    ? loadStorageAnalyzer()
    : null;

  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => {
      if (mounted) setAnalyzer(() => component);
    }).catch(() => {
      if (mounted) setError('The folder analyzer could not be loaded.');
    });
    return () => {
      mounted = false;
    };
  }, [request]);

  if (Analyzer !== null) return <Analyzer {...props} />;
  return (
    <div class="dialog-backdrop" role="presentation">
      <section ref={modalRef} tabIndex={-1} class="dialog" role="dialog" aria-modal="true" aria-labelledby="storage-analyzer-loading-title">
        <header><h2 id="storage-analyzer-loading-title">Folder size analysis</h2><button class="icon-button" type="button" data-modal-autofocus onClick={props.onClose} aria-label="Close storage analysis"><Icon name="close" /></button></header>
        <InlineError message={error} />
        <div class="detail-loading" role="status" aria-live="polite">
          <Icon name={error === null ? 'search' : 'warning'} size={28} />
          <span>{error === null ? 'Opening the analyzer…' : 'The file manager is still available.'}</span>
          {error !== null && <button class="button button--primary" type="button" onClick={() => setError(null)}>Try again</button>}
        </div>
      </section>
    </div>
  );
}
