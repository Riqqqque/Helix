import { useCallback, useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { ApiError } from './api';
import { createDirectory, createFile, getDirectory, readTextFile, renameFile, trashFile, writeTextFile, type DirectoryListing, type FileEntry, type TextFile } from './control-api';
import { InlineError, ProgressBar } from './dashboard-ui';
import { droppedTransferFiles, MAX_STORAGE_UPLOAD_BYTES, uploadHostFile } from './file-upload';
import { formatBytes, formatTimestamp } from './format';
import { Icon } from './icons';
import { Dialog } from './modal';
import { StorageAnalyzerRoute } from './storage-analyzer-route';
import type { StorageAnalysisMode } from './storage-analysis-api';

const MAX_TEXT_EDITOR_BYTES = 4 * 1024 * 1024;
const pageSizes = [25, 50, 100, 200] as const;
const textExtensions = new Set([
  'cfg', 'conf', 'css', 'csv', 'env', 'html', 'ini', 'java', 'js', 'json', 'json5',
  'log', 'md', 'mcmeta', 'properties', 'ps1', 'py', 'rs', 'sh', 'toml', 'ts', 'tsx',
  'txt', 'xml', 'yaml', 'yml',
]);
const textNames = new Set(['dockerfile', 'license', 'makefile', 'readme']);

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : 'Helix could not complete that request.';
}

function isSessionError(error: unknown): boolean {
  return error instanceof ApiError && (error.status === 401 || error.code === 'csrf_rejected');
}

function extension(name: string): string | null {
  const dot = name.lastIndexOf('.');
  return dot > 0 && dot < name.length - 1 ? name.slice(dot + 1).toLowerCase() : null;
}

export function isTextEditable(entry: FileEntry): boolean {
  if (entry.kind !== 'file' || entry.sizeBytes > MAX_TEXT_EDITOR_BYTES) return false;
  const suffix = extension(entry.name);
  return (suffix !== null && textExtensions.has(suffix)) || textNames.has(entry.name.toLowerCase());
}

export function fileTypeLabel(entry: FileEntry): string {
  if (entry.kind === 'directory') return 'Folder';
  if (entry.kind === 'symlink') return 'Symbolic link';
  if (entry.kind === 'other') return 'Special item';
  const suffix = extension(entry.name);
  if (suffix === null) return 'File';
  const known: Record<string, string> = {
    avi: 'AVI video', gif: 'GIF image', jpeg: 'JPEG image', jpg: 'JPEG image',
    mkv: 'MKV video', mov: 'QuickTime video', mp3: 'MP3 audio', mp4: 'MP4 video',
    pdf: 'PDF document', png: 'PNG image', wav: 'WAV audio', webm: 'WebM video',
    webp: 'WebP image', jar: 'Java archive', zip: 'ZIP archive',
  };
  return known[suffix] ?? `${suffix.toUpperCase()} file`;
}

export interface FileManagerProps {
  csrfToken: string;
  onSessionExpired: () => void;
  initialPath: string;
  analysis?: { path: string; mode: StorageAnalysisMode } | null;
  onAnalysisClose?: () => void;
}

export function FileManager({ csrfToken, onSessionExpired, initialPath, analysis = null, onAnalysisClose }: FileManagerProps) {
  const [path, setPath] = useState(initialPath);
  const [listing, setListing] = useState<DirectoryListing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState('');
  const [pageSize, setPageSize] = useState<number>(50);
  const [pageIndex, setPageIndex] = useState(0);
  const [cursorHistory, setCursorHistory] = useState<Array<string | null>>([null]);
  const [createKind, setCreateKind] = useState<'directory' | 'file' | null>(null);
  const [createName, setCreateName] = useState('');
  const [renameTarget, setRenameTarget] = useState<FileEntry | null>(null);
  const [renameName, setRenameName] = useState('');
  const [trashTarget, setTrashTarget] = useState<FileEntry | null>(null);
  const [editor, setEditor] = useState<TextFile | null>(null);
  const [editorContent, setEditorContent] = useState('');
  const [busy, setBusy] = useState(false);
  const [analysisOpen, setAnalysisOpen] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [uploadPercent, setUploadPercent] = useState<number | null>(null);
  const [uploadLabel, setUploadLabel] = useState<string | null>(null);
  const fileInput = useRef<HTMLInputElement | null>(null);
  const activeLoad = useRef<AbortController | null>(null);
  const pageSizeRef = useRef(50);

  const load = useCallback(async (
    nextPath: string,
    cursor: string | null = null,
    nextPageIndex = 0,
    limit = pageSizeRef.current,
  ): Promise<void> => {
    activeLoad.current?.abort();
    const controller = new AbortController();
    activeLoad.current = controller;
    setLoading(true);
    setError(null);
    try {
      const next = await getDirectory(nextPath, csrfToken, cursor, limit, controller.signal);
      if (controller.signal.aborted) return;
      setListing(next);
      setPath(next.path);
      setPageIndex(nextPageIndex);
      setSearch('');
    } catch (requestError) {
      if (controller.signal.aborted) return;
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      if (activeLoad.current === controller) {
        activeLoad.current = null;
        setLoading(false);
      }
    }
  }, [csrfToken, onSessionExpired]);

  const navigate = useCallback((nextPath: string): void => {
    setCursorHistory([null]);
    void load(nextPath, null, 0);
  }, [load]);

  useEffect(() => {
    setCursorHistory([null]);
    void load(initialPath, null, 0);
    return () => activeLoad.current?.abort();
  }, [initialPath, load]);

  const entries = useMemo(() => {
    const query = search.trim().toLowerCase();
    return (listing?.entries ?? []).filter((entry) => query.length === 0 || entry.name.toLowerCase().includes(query));
  }, [listing, search]);
  const crumbs = path === '/' ? ['/'] : ['/', ...path.split('/').filter(Boolean)];
  const totalPages = Math.max(1, Math.ceil((listing?.totalEntries ?? 0) / pageSize));

  const mutate = async (operation: () => Promise<unknown>, after: () => void): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      await operation();
      after();
      await load(path, cursorHistory[pageIndex] ?? null, pageIndex);
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };

  const openEntry = async (entry: FileEntry): Promise<void> => {
    if (entry.kind === 'directory') {
      navigate(entry.path);
      return;
    }
    if (!isTextEditable(entry)) return;
    setBusy(true);
    setError(null);
    try {
      const file = await readTextFile(entry.path, csrfToken);
      setEditor(file);
      setEditorContent(file.content);
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };

  const uploadFiles = async (files: File[]): Promise<void> => {
    if (!listing?.writable || files.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      for (const [index, file] of files.entries()) {
        setUploadLabel(files.length === 1 ? file.name : `${file.name} (${index + 1} of ${files.length})`);
        setUploadPercent(0);
        await uploadHostFile({
          file,
          purpose: 'storage',
          parent: path,
          csrfToken,
          onProgress: setUploadPercent,
        });
      }
      await load(path, cursorHistory[pageIndex] ?? null, pageIndex);
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
      setUploadPercent(null);
      setUploadLabel(null);
      setDragging(false);
      if (fileInput.current !== null) fileInput.current.value = '';
    }
  };

  const nextPage = (): void => {
    if (listing?.nextCursor === null || listing?.nextCursor === undefined || !listing.hasMore) return;
    const nextCursor = listing.nextCursor;
    const nextIndex = pageIndex + 1;
    setCursorHistory((current) => [...current.slice(0, nextIndex), nextCursor]);
    void load(path, nextCursor, nextIndex);
  };

  const previousPage = (): void => {
    if (pageIndex === 0) return;
    const previousIndex = pageIndex - 1;
    void load(path, cursorHistory[previousIndex] ?? null, previousIndex);
  };

  const changePageSize = (limit: number): void => {
    pageSizeRef.current = limit;
    setPageSize(limit);
    setCursorHistory([null]);
    void load(path, null, 0, limit);
  };
  const closeAnalysis = (): void => {
    setAnalysisOpen(false);
    onAnalysisClose?.();
  };
  const analysisTarget = analysis ?? (analysisOpen ? { path, mode: 'quick' as const } : null);
  const canDrop =
    listing?.writable === true &&
    !busy &&
    createKind === null &&
    renameTarget === null &&
    trashTarget === null &&
    editor === null &&
    analysisTarget === null;

  return (
    <section
      class={`file-manager surface${dragging ? ' is-drop-target' : ''}`}
      aria-busy={loading}
      onDragEnter={(event) => {
        event.preventDefault();
        if (canDrop) setDragging(true);
      }}
      onDragOver={(event) => {
        event.preventDefault();
        if (event.dataTransfer !== null) event.dataTransfer.dropEffect = canDrop ? 'copy' : 'none';
      }}
      onDragLeave={(event) => {
        if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
        setDragging(false);
      }}
      onDrop={(event) => {
        event.preventDefault();
        setDragging(false);
        if (!canDrop) return;
        try {
          const files = droppedTransferFiles(event.dataTransfer);
          if (files.length > 0) void uploadFiles(files);
        } catch (dropError) {
          setError(describeError(dropError));
        }
      }}
    >
      {dragging && listing?.writable === true && (
        <div class="file-drop-overlay">
          <Icon name="plus" size={22} />
          <strong>Drop files to upload</strong>
          <span>Up to {formatBytes(MAX_STORAGE_UPLOAD_BYTES)} each. Folders are rejected.</span>
        </div>
      )}
      <div class="file-toolbar">
        <div class="breadcrumbs" aria-label="Current path">
          {crumbs.map((crumb, index) => {
            const target = index === 0 ? '/' : `/${crumbs.slice(1, index + 1).join('/')}`;
            return <span key={`${crumb}-${index}`}><button type="button" onClick={() => navigate(target)}>{crumb}</button>{index < crumbs.length - 1 && <Icon name="chevron" size={12} />}</span>;
          })}
        </div>
        <div class="file-actions">
          <label class="search-box"><Icon name="search" size={16} /><input value={search} onInput={(event) => setSearch(event.currentTarget.value)} placeholder="Filter this page" aria-label="Filter this page" /></label>
          <button class="button button--quiet" type="button" onClick={() => void load(path, cursorHistory[pageIndex] ?? null, pageIndex)} disabled={loading}><Icon name="refresh" size={16} />Refresh</button>
          <button class="button button--quiet" type="button" disabled={listing === null || loading} onClick={() => setAnalysisOpen(true)} title="Find the largest files and calculate folder totals"><Icon name="performance" size={16} />Analyze folder</button>
          <button class="button" type="button" disabled={!listing?.writable || busy} onClick={() => { setCreateName(''); setCreateKind('directory'); }}><Icon name="folder" size={16} />New folder</button>
          <button class="button" type="button" disabled={!listing?.writable || busy} onClick={() => { setCreateName(''); setCreateKind('file'); }}><Icon name="file" size={16} />New file</button>
          <button class="button" type="button" disabled={!listing?.writable || busy} onClick={() => fileInput.current?.click()}><Icon name="plus" size={16} />Upload</button>
          <input ref={fileInput} class="sr-only" type="file" multiple disabled={!listing?.writable || busy} onChange={(event) => { const files = [...(event.currentTarget.files ?? [])]; if (files.length > 0) void uploadFiles(files); }} />
        </div>
      </div>
      <InlineError message={error} />
      {uploadPercent !== null && (
        <div class="file-upload-progress" role="status">
          <span>Uploading {uploadLabel}</span>
          <ProgressBar value={uploadPercent} />
        </div>
      )}
      <div class="file-table-wrap">
        <table class="data-table file-table">
          <thead><tr><th>Name</th><th>Type</th><th>Size</th><th>Modified</th><th>Mode</th><th><span class="sr-only">Actions</span></th></tr></thead>
          <tbody>
            {listing?.parent !== null && listing !== null && <tr class="file-row file-row--parent" onDblClick={() => listing.parent !== null && navigate(listing.parent)}><td><button type="button" onClick={() => listing.parent !== null && navigate(listing.parent)}><Icon name="folder" />..</button></td><td>Parent folder</td><td>—</td><td>—</td><td>—</td><td /></tr>}
            {entries.map((entry) => {
              const editable = isTextEditable(entry);
              return <tr class="file-row" key={entry.path} onDblClick={() => void openEntry(entry)}>
                <td>{entry.kind === 'directory' || editable ? <button class="file-name" type="button" onClick={() => void openEntry(entry)} title={entry.kind === 'directory' ? `Open ${entry.name}` : `Edit text contents of ${entry.name}`}><Icon name={entry.kind === 'directory' ? 'folder' : 'file'} /><span>{entry.name}</span>{entry.restricted && <small>restricted</small>}</button> : <span class="file-name file-name--static"><Icon name="file" /><span>{entry.name}</span>{entry.restricted && <small>restricted</small>}</span>}</td>
                <td><span class="file-type">{fileTypeLabel(entry)}</span></td>
                <td>{entry.kind === 'directory' ? <span class="file-size-unknown" title="Run Analyze folder to calculate recursive folder sizes without slowing normal browsing">—</span> : formatBytes(entry.sizeBytes)}</td>
                <td>{entry.modifiedUnixMs === null ? '—' : formatTimestamp(entry.modifiedUnixMs)}</td>
                <td><code>{entry.permissions}</code></td>
                <td><div class="row-actions">{editable && <button class="icon-button" type="button" onClick={() => void openEntry(entry)} aria-label={`Edit text contents of ${entry.name}`} title="Edit text contents"><Icon name="note" size={16} /></button>}<button class="icon-button" type="button" disabled={!entry.writable || busy} onClick={() => { setRenameTarget(entry); setRenameName(entry.name); }} aria-label={`Rename ${entry.name}`} title="Rename"><Icon name="edit" size={16} /></button><button class="icon-button icon-button--danger" type="button" disabled={!entry.writable || busy} onClick={() => setTrashTarget(entry)} aria-label={`Move ${entry.name} to trash`} title="Move to recoverable trash"><Icon name="trash" size={16} /></button></div></td>
              </tr>;
            })}
          </tbody>
        </table>
        {loading && <div class="table-state table-state--loading"><Icon name="refresh" class="is-spinning" size={17} />Reading {path}…</div>}
        {!loading && entries.length === 0 && <div class="table-state">{search.length > 0 ? 'No names on this page match the filter.' : 'This folder is empty.'}</div>}
      </div>
      <div class="file-pagination">
        <span>{listing === null ? 'Waiting for directory data' : `${listing.totalEntries.toLocaleString()} item${listing.totalEntries === 1 ? '' : 's'} · Page ${Math.min(pageIndex + 1, totalPages)} of ${totalPages}`}</span>
        <label><span>Rows</span><select value={pageSize} disabled={loading} onChange={(event) => changePageSize(Number(event.currentTarget.value))}>{pageSizes.map((size) => <option value={size} key={size}>{size}</option>)}</select></label>
        <div><button class="button button--quiet" type="button" disabled={loading || pageIndex === 0} onClick={previousPage}><Icon name="back" size={14} />Previous</button><button class="button button--quiet" type="button" disabled={loading || listing?.hasMore !== true} onClick={nextPage}>Next<Icon name="chevron" size={14} /></button></div>
      </div>
      {listing !== null && listing.omittedEntries > 0 && <p class="table-note">{listing.omittedEntries.toLocaleString()} entries could not be read or safely represented. The count above includes every representable name.</p>}

      {analysisTarget !== null && <StorageAnalyzerRoute path={analysisTarget.path} initialMode={analysisTarget.mode} csrfToken={csrfToken} onClose={closeAnalysis} onNavigate={(target) => { closeAnalysis(); navigate(target); }} onSessionExpired={onSessionExpired} />}
      {createKind !== null && <Dialog title={createKind === 'directory' ? 'Create folder' : 'Create file'} onClose={() => setCreateKind(null)}><form class="dialog-form" onSubmit={(event) => { event.preventDefault(); const operation = createKind === 'directory' ? createDirectory : createFile; void mutate(() => operation(path, createName, csrfToken), () => setCreateKind(null)); }}><label><span>Name</span><input autofocus required value={createName} onInput={(event) => setCreateName(event.currentTarget.value)} /></label><div class="dialog-actions"><button class="button button--quiet" type="button" onClick={() => setCreateKind(null)}>Cancel</button><button class="button button--primary" type="submit" disabled={busy || createName.trim().length === 0}>Create</button></div></form></Dialog>}
      {renameTarget !== null && <Dialog title={`Rename ${renameTarget.name}`} onClose={() => setRenameTarget(null)}><form class="dialog-form" onSubmit={(event) => { event.preventDefault(); void mutate(() => renameFile(renameTarget.path, renameName, csrfToken), () => setRenameTarget(null)); }}><label><span>New name</span><input autofocus required value={renameName} onInput={(event) => setRenameName(event.currentTarget.value)} /></label><div class="dialog-actions"><button class="button button--quiet" type="button" onClick={() => setRenameTarget(null)}>Cancel</button><button class="button button--primary" type="submit" disabled={busy || renameName.trim().length === 0}>Rename</button></div></form></Dialog>}
      {trashTarget !== null && <Dialog title="Move to trash?" onClose={() => setTrashTarget(null)}><div class="dialog-copy"><p><strong>{trashTarget.name}</strong> will move into this drive’s protected <code>.helix-trash</code> folder. Helix never permanently deletes it from this action.</p></div><div class="dialog-actions"><button class="button button--quiet" type="button" onClick={() => setTrashTarget(null)}>Cancel</button><button class="button button--danger" type="button" disabled={busy} onClick={() => void mutate(() => trashFile(trashTarget.path, csrfToken), () => setTrashTarget(null))}>Move to trash</button></div></Dialog>}
      {editor !== null && <Dialog title={editor.path.split('/').at(-1) ?? editor.path} onClose={() => setEditor(null)} wide flush><div class="editor-path">UTF-8 text · {editor.path} · {formatBytes(new TextEncoder().encode(editorContent).length)} / 4 MiB</div><textarea class="text-editor" spellcheck={false} value={editorContent} onInput={(event) => setEditorContent(event.currentTarget.value)} /><div class="dialog-actions"><button class="button button--quiet" type="button" onClick={() => setEditor(null)}>Close</button><button class="button button--primary" type="button" disabled={busy || editorContent === editor.content || new TextEncoder().encode(editorContent).length > MAX_TEXT_EDITOR_BYTES} onClick={() => { setBusy(true); void writeTextFile(editor, editorContent, csrfToken).then((saved) => { setEditor(saved); setEditorContent(saved.content); }).catch((requestError: unknown) => setError(describeError(requestError))).finally(() => setBusy(false)); }}>Save changes</button></div></Dialog>}
    </section>
  );
}
