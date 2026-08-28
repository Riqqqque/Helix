import { useEffect, useState } from 'preact/hooks';
import type { DockerInventoryPanel as DockerInventoryPanelComponent } from './docker-panel';

type DockerPanel = typeof DockerInventoryPanelComponent;
let loadedPanel: DockerPanel | null = null;
let pendingPanel: Promise<DockerPanel> | null = null;

export function loadDockerPanel(): Promise<DockerPanel> {
  if (loadedPanel !== null) return Promise.resolve(loadedPanel);
  pendingPanel ??= import('./docker-panel').then((module) => {
    loadedPanel = module.DockerInventoryPanel;
    return loadedPanel;
  }).catch((error: unknown) => {
    pendingPanel = null;
    throw error;
  });
  return pendingPanel;
}

export function preloadDockerPanel(): void {
  void loadDockerPanel().catch(() => undefined);
}

export function DockerInventoryRoute(props: Parameters<DockerPanel>[0]) {
  const [Panel, setPanel] = useState<DockerPanel | null>(() => loadedPanel);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Panel === null && error === null
    ? loadDockerPanel()
    : null;
  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => { if (mounted) setPanel(() => component); }).catch(() => {
      if (mounted) setError('Docker inventory could not be loaded.');
    });
    return () => { mounted = false; };
  }, [request]);
  if (Panel !== null) return <Panel {...props} />;
  return <div class="docker-empty" aria-busy={error === null}>{error ?? 'Loading containers…'}</div>;
}
