import { useEffect, useState } from 'preact/hooks';
import { InlineError } from './dashboard-ui';
import type { FileManager as FileManagerComponent, FileManagerProps } from './file-manager';
import { Icon } from './icons';

type FileManagerComponentType = typeof FileManagerComponent;

let loadedFileManager: FileManagerComponentType | null = null;
let pendingFileManager: Promise<FileManagerComponentType> | null = null;

export function loadFileManager(): Promise<FileManagerComponentType> {
  if (loadedFileManager !== null) return Promise.resolve(loadedFileManager);
  pendingFileManager ??= import('./file-manager').then((module) => {
    loadedFileManager = module.FileManager;
    return loadedFileManager;
  }).catch((error: unknown) => {
    pendingFileManager = null;
    throw error;
  });
  return pendingFileManager;
}

export function preloadFileManager(): void {
  void loadFileManager().catch(() => undefined);
}

export function FileManagerRoute(props: FileManagerProps) {
  const [Manager, setManager] = useState<FileManagerComponentType | null>(() => loadedFileManager);
  const [error, setError] = useState<string | null>(null);
  const request = typeof document !== 'undefined' && Manager === null && error === null
    ? loadFileManager()
    : null;

  useEffect(() => {
    if (request === null) return;
    let mounted = true;
    void request.then((component) => {
      if (mounted) setManager(() => component);
    }).catch(() => {
      if (mounted) setError('The file manager could not be loaded.');
    });
    return () => {
      mounted = false;
    };
  }, [request]);

  if (Manager !== null) return <Manager {...props} />;
  return (
    <section class="file-manager surface" aria-busy={error === null}>
      <InlineError message={error} />
      <div class="detail-loading" role="status" aria-live="polite">
        <Icon name={error === null ? 'folder' : 'warning'} size={26} />
        <span>{error === null ? 'Loading the file manager…' : 'Drive information remains available above.'}</span>
        {error !== null && <button class="button button--primary" type="button" onClick={() => setError(null)}>Try again</button>}
      </div>
    </section>
  );
}
