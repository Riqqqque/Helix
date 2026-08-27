import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { ApiError } from './api';
import { trashFile } from './control-api';
import { InlineError } from './dashboard-ui';
import { formatBytes, formatTimestamp } from './format';
import { Icon } from './icons';
import { InfoTip } from './info-tip';
import { Dialog } from './modal';
import { useModalFocus } from './modal-focus';
import {
  cancelStorageAnalysis,
  getStorageAnalysisStatus,
  startStorageAnalysis,
  type StorageAnalysisFile,
  type StorageAnalysisFolder,
  type StorageAnalysisJobState,
  type StorageAnalysisMode,
  type StorageAnalysisResult,
  type StorageAnalysisStatus,
} from './storage-analysis-api';
import './storage-analyzer.css';

const POLL_INTERVAL_MS = 1_000;
const RESULT_PAGE_SIZE = 50;
const countFormat = new Intl.NumberFormat();

export type StorageAnalysisSort = 'size-desc' | 'size-asc' | 'name-asc' | 'path-asc';
type ResultList = 'files' | 'recursive' | 'immediate';

export interface StorageAnalyzerProps {
  path: string;
  csrfToken: string;
  onClose: () => void;
  onNavigate: (path: string) => void;
  onSessionExpired: () => void;
}

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : 'Helix could not complete the storage analysis.';
}

function isSessionError(error: unknown): boolean {
  return error instanceof ApiError && (error.status === 401 || error.code === 'csrf_rejected');
}

function isActiveState(state: StorageAnalysisJobState): boolean {
  return state === 'queued' || state === 'running';
}

function compareText(left: string, right: string): number {
  return left.localeCompare(right, undefined, { numeric: true, sensitivity: 'base' });
}

export function sortStorageAnalysisFiles(
  files: readonly StorageAnalysisFile[],
  sort: StorageAnalysisSort,
): StorageAnalysisFile[] {
  return [...files].sort((left, right) => {
    if (sort === 'name-asc') {
      return compareText(left.name, right.name) || compareText(left.path, right.path);
    }
    if (sort === 'path-asc') return compareText(left.path, right.path);
    const size = left.bytes === right.bytes ? 0 : left.bytes < right.bytes ? -1 : 1;
    return (sort === 'size-asc' ? size : -size) || compareText(left.path, right.path);
  });
}

export function sortStorageAnalysisFolders(
  folders: readonly StorageAnalysisFolder[],
  metric: 'recursive' | 'immediate',
  sort: StorageAnalysisSort,
): StorageAnalysisFolder[] {
  return [...folders].sort((left, right) => {
    if (sort === 'name-asc') {
      return compareText(left.name, right.name) || compareText(left.path, right.path);
    }
    if (sort === 'path-asc') return compareText(left.path, right.path);
    const leftBytes = metric === 'recursive' ? left.recursiveBytes : left.immediateBytes;
    const rightBytes = metric === 'recursive' ? right.recursiveBytes : right.immediateBytes;
    const size = leftBytes === rightBytes ? 0 : leftBytes < rightBytes ? -1 : 1;
    return (sort === 'size-asc' ? size : -size) || compareText(left.path, right.path);
  });
}

export function storageResultNavigationPath(
  result: StorageAnalysisFile | StorageAnalysisFolder,
): string {
  if (result.type === 'directory') return result.path;
  const separator = result.path.lastIndexOf('/');
  return separator <= 0 ? '/' : result.path.slice(0, separator);
}

function skippedTotal(result: StorageAnalysisResult): number {
  const skipped = result.skipped;
  return skipped.restrictedPaths + skipped.symbolicLinks + skipped.otherFilesystems +
    skipped.specialFiles + skipped.depthLimitedDirectories + skipped.unrepresentableNames;
}

function omittedTotal(result: StorageAnalysisResult): number {
  return result.fileResultsOmitted + result.recursiveFolderResultsOmitted +
    result.immediateFolderResultsOmitted + result.responseResultsOmitted;
}

function stopReasonLabel(result: StorageAnalysisResult): string | null {
  if (result.stopReason === 'entry_limit') return `The ${countFormat.format(result.limits.maxEntries)}-entry safety limit was reached.`;
  if (result.stopReason === 'duration_limit') return `The ${countFormat.format(result.limits.maxDurationMs / 1_000)}-second safety limit was reached.`;
  if (result.stopReason === 'cancelled') return 'The scan was stopped at your request.';
  return null;
}

function ResultNotice({ result }: { result: StorageAnalysisResult }) {
  const skipped = skippedTotal(result);
  const omitted = omittedTotal(result);
  return (
    <div
      class={'storage-analysis-notice ' + (result.truncated ? 'is-partial' : 'is-complete')}
      role="status"
    >
      <Icon name={result.truncated ? 'warning' : 'check'} size={17} />
      <div>
        <strong>{result.truncated ? 'Partial scan coverage' : 'Full scan coverage'}</strong>
        <span>
          {stopReasonLabel(result) ?? (
            result.errors.total > 0
              ? countFormat.format(result.errors.total) + ' filesystem errors prevented complete totals.'
              : 'Every eligible entry under this folder was considered.'
          )}
          {skipped > 0 && ' ' + countFormat.format(skipped) + ' entries were excluded by safety policy or file type.'}
          {omitted > 0 && ' Only the largest ranking rows are retained; this does not change which shown files are largest among the scanned entries.'}
        </span>
      </div>
    </div>
  );
}

function ResultDetails({ result }: { result: StorageAnalysisResult }) {
  const skipped = result.skipped;
  const errors = result.errors;
  return (
    <details class="storage-analysis-details">
      <summary>Scan coverage and limits</summary>
      <div>
        <dl>
          <div><dt>Permission denied</dt><dd>{countFormat.format(errors.permissionDenied)}</dd></div>
          <div><dt>Changed during scan</dt><dd>{countFormat.format(errors.filesystemRaces)}</dd></div>
          <div><dt>Metadata failures</dt><dd>{countFormat.format(errors.metadataFailures)}</dd></div>
          <div><dt>Size overflows</dt><dd>{countFormat.format(errors.sizeOverflows)}</dd></div>
          <div><dt>Protected paths</dt><dd>{countFormat.format(skipped.restrictedPaths)}</dd></div>
          <div><dt>Symbolic links</dt><dd>{countFormat.format(skipped.symbolicLinks)}</dd></div>
          <div><dt>Other filesystems</dt><dd>{countFormat.format(skipped.otherFilesystems)}</dd></div>
          <div><dt>Special files</dt><dd>{countFormat.format(skipped.specialFiles)}</dd></div>
          <div><dt>Depth-limited folders</dt><dd>{countFormat.format(skipped.depthLimitedDirectories)}</dd></div>
          <div><dt>Unrepresentable names</dt><dd>{countFormat.format(skipped.unrepresentableNames)}</dd></div>
        </dl>
        <p>
          {result.limits.mode === 'thorough' ? 'Thorough' : 'Quick'} · metadata only · {countFormat.format(result.limits.maxEntries)} entries · {countFormat.format(result.limits.maxDurationMs / 1_000)} seconds · depth {countFormat.format(result.limits.maxDepth)} · no symlinks · target filesystem only
        </p>
      </div>
    </details>
  );
}

function Completeness({ complete, label }: { complete: boolean; label: string }) {
  return (
    <span class={'storage-analysis-coverage' + (complete ? '' : ' is-partial')}>
      {complete ? label + ' complete' : label + ' partial'}
    </span>
  );
}

function FileRows({
  files,
  onNavigate,
  onTrashRequest,
}: {
  files: readonly StorageAnalysisFile[];
  onNavigate: (path: string) => void;
  onTrashRequest?: (file: StorageAnalysisFile) => void;
}) {
  return (
    <div class="storage-analysis-table-wrap">
      <table class="storage-analysis-table">
        <thead><tr><th>File</th><th>Apparent size</th><th>Type</th><th><span class="sr-only">Actions</span></th></tr></thead>
        <tbody>{files.map((file) => (
          <tr key={file.path}>
            <td><span class="storage-analysis-entry"><Icon name="file" size={16} /><span><strong>{file.name}</strong><code>{file.path}</code></span></span></td>
            <td><strong>{formatBytes(file.bytes)}</strong></td>
            <td>File</td>
            <td><span class="storage-analysis-row-actions"><button class="button button--quiet" type="button" onClick={() => onNavigate(storageResultNavigationPath(file))}><Icon name="folder" size={14} />Show in files</button>{onTrashRequest !== undefined && <button class="button button--danger-quiet" type="button" onClick={() => onTrashRequest(file)}><Icon name="trash" size={14} />Move to trash</button>}</span></td>
          </tr>
        ))}</tbody>
      </table>
    </div>
  );
}

function FolderRows({
  folders,
  metric,
  onNavigate,
}: {
  folders: readonly StorageAnalysisFolder[];
  metric: 'recursive' | 'immediate';
  onNavigate: (path: string) => void;
}) {
  return (
    <div class="storage-analysis-table-wrap">
      <table class="storage-analysis-table storage-analysis-table--folders">
        <thead><tr><th>Folder</th><th>{metric === 'recursive' ? 'Recursive total' : 'Immediate total'}</th><th>{metric === 'recursive' ? 'Immediate' : 'Recursive'}</th><th>Coverage</th><th><span class="sr-only">Action</span></th></tr></thead>
        <tbody>{folders.map((folder) => (
          <tr key={folder.path}>
            <td><span class="storage-analysis-entry"><Icon name="folder" size={16} /><span><strong>{folder.name}</strong><code>{folder.path}</code></span></span></td>
            <td><strong>{formatBytes(metric === 'recursive' ? folder.recursiveBytes : folder.immediateBytes)}</strong></td>
            <td>{formatBytes(metric === 'recursive' ? folder.immediateBytes : folder.recursiveBytes)}</td>
            <td><span class="storage-analysis-coverages"><Completeness complete={folder.immediateComplete} label="Direct" /><Completeness complete={folder.recursiveComplete} label="Tree" /></span></td>
            <td><button class="button button--quiet" type="button" onClick={() => onNavigate(storageResultNavigationPath(folder))}><Icon name="folder" size={14} />Open folder</button></td>
          </tr>
        ))}</tbody>
      </table>
    </div>
  );
}

export function StorageAnalysisResults({
  result,
  onNavigate,
  onTrashRequest,
  hiddenPaths = new Set<string>(),
}: {
  result: StorageAnalysisResult;
  onNavigate: (path: string) => void;
  onTrashRequest?: (file: StorageAnalysisFile) => void;
  hiddenPaths?: ReadonlySet<string>;
}) {
  const [list, setList] = useState<ResultList>('files');
  const [sort, setSort] = useState<StorageAnalysisSort>('size-desc');
  const [page, setPage] = useState(0);
  const rows = useMemo(() => {
    if (list === 'files') {
      return sortStorageAnalysisFiles(
        result.largestFiles.filter((file) => !hiddenPaths.has(file.path)),
        sort,
      );
    }
    const folders = list === 'recursive'
      ? result.largestFoldersByRecursiveBytes
      : result.largestFoldersByImmediateBytes;
    return sortStorageAnalysisFolders(folders, list, sort);
  }, [hiddenPaths, list, result, sort]);
  const pageCount = Math.max(1, Math.ceil(rows.length / RESULT_PAGE_SIZE));
  const pageRows = rows.slice(page * RESULT_PAGE_SIZE, (page + 1) * RESULT_PAGE_SIZE);
  useEffect(() => setPage(0), [list, sort, result]);
  useEffect(() => {
    if (page >= pageCount) setPage(pageCount - 1);
  }, [page, pageCount]);
  const omitted = list === 'files'
    ? result.fileResultsOmitted
    : list === 'recursive'
      ? result.recursiveFolderResultsOmitted
      : result.immediateFolderResultsOmitted;

  return (
    <div class="storage-analysis-results">
      <ResultNotice result={result} />
      <div class="storage-analysis-summary">
        <div><span>Apparent bytes analyzed <InfoTip text="Logical file length from metadata. Sparse allocation is not measured, and hard links can be counted once per directory entry." /></span><strong>{formatBytes(result.apparentBytesScanned)}</strong></div>
        <div><span>Filesystem errors <InfoTip text="Entries Helix could not inspect because permissions, concurrent file changes, metadata failures, or size overflow made a trustworthy total impossible." /></span><strong>{countFormat.format(result.errors.total)}</strong></div>
        <div><span>Skipped entries <InfoTip text="Protected paths, symbolic links, other filesystems, special files, depth-limited folders, and names the API cannot safely represent are never followed or guessed." /></span><strong>{countFormat.format(skippedTotal(result))}</strong></div>
        <div><span>Ranking rows omitted <InfoTip text="Helix bounds every result list and the full response size. Omitted rows are counted even though they are not shown." /></span><strong>{countFormat.format(omittedTotal(result))}</strong></div>
      </div>
      <ResultDetails result={result} />
      <div class="storage-analysis-definitions"><span>Folder trees <InfoTip text="Recursive totals include regular files in the folder and every scanned descendant folder." /></span><span>Direct contents <InfoTip text="Immediate totals include only regular files directly inside each folder, not files in child folders." /></span></div>
      <div class="storage-analysis-result-tools">
        <div class="storage-analysis-tabs" role="group" aria-label="Analysis results">
          <button type="button" class={list === 'files' ? 'is-active' : ''} aria-pressed={list === 'files'} onClick={() => setList('files')}>Files <span>{result.largestFiles.length}</span></button>
          <button type="button" class={list === 'recursive' ? 'is-active' : ''} aria-pressed={list === 'recursive'} onClick={() => setList('recursive')}>Folder trees <span>{result.largestFoldersByRecursiveBytes.length}</span></button>
          <button type="button" class={list === 'immediate' ? 'is-active' : ''} aria-pressed={list === 'immediate'} onClick={() => setList('immediate')}>Direct contents <span>{result.largestFoldersByImmediateBytes.length}</span></button>
        </div>
        <label class="storage-analysis-sort"><span>Sort</span><select value={sort} onChange={(event) => setSort(event.currentTarget.value as StorageAnalysisSort)}><option value="size-desc">Largest first</option><option value="size-asc">Smallest first</option><option value="name-asc">Name A–Z</option><option value="path-asc">Path A–Z</option></select></label>
      </div>
      {rows.length === 0 ? (
        <div class="storage-analysis-empty">
          <Icon name="search" size={24} />
          <strong>No ranking rows returned</strong>
          <span>{result.responseResultsOmitted > 0 ? 'Rows were removed to keep the response within its safety limit.' : 'No matching regular files or folders were found.'}</span>
        </div>
      ) : list === 'files' ? (
        <FileRows files={pageRows as StorageAnalysisFile[]} onNavigate={onNavigate} {...(onTrashRequest === undefined ? {} : { onTrashRequest })} />
      ) : (
        <FolderRows folders={pageRows as StorageAnalysisFolder[]} metric={list} onNavigate={onNavigate} />
      )}
      {rows.length > RESULT_PAGE_SIZE && <div class="storage-analysis-pagination"><span>Showing {countFormat.format(page * RESULT_PAGE_SIZE + 1)}–{countFormat.format(Math.min(rows.length, (page + 1) * RESULT_PAGE_SIZE))} of {countFormat.format(rows.length)} retained rows</span><div><button class="button button--quiet" type="button" disabled={page === 0} onClick={() => setPage((value) => Math.max(0, value - 1))}><Icon name="back" size={14} />Previous</button><button class="button button--quiet" type="button" disabled={page + 1 >= pageCount} onClick={() => setPage((value) => Math.min(pageCount - 1, value + 1))}>Next<Icon name="chevron" size={14} /></button></div></div>}
      {omitted > 0 && <p class="storage-analysis-omitted">{countFormat.format(omitted)} additional {list === 'files' ? 'files' : 'folders'} were outside this bounded ranking.</p>}
    </div>
  );
}

function StatusHeadline({ status, waiting }: { status: StorageAnalysisStatus | null; waiting: boolean }) {
  if (waiting || status === null) {
    return <><strong>Starting bounded scan…</strong><span>Waiting for the first progress snapshot.</span></>;
  }
  if (status.cancelRequested && isActiveState(status.state)) {
    return <><strong>Stopping safely…</strong><span>Helix will finish the current filesystem operation, then preserve partial results.</span></>;
  }
  if (status.state === 'queued') {
    return <><strong>Queued</strong><span>Waiting for one of two storage-analysis worker slots.</span></>;
  }
  if (status.state === 'running') {
    return <><strong>Reading metadata</strong><span>Folder contents can change during the scan, so totals remain provisional.</span></>;
  }
  if (status.state === 'complete') {
    return <><strong>Analysis complete</strong><span>Finished {status.finishedUnixMs === null ? 'safely' : formatTimestamp(status.finishedUnixMs)}.</span></>;
  }
  if (status.state === 'cancelled') {
    return <><strong>Analysis stopped</strong><span>Partial results gathered before cancellation are still available.</span></>;
  }
  return <><strong>Analysis failed safely</strong><span>No file contents or live data were changed.</span></>;
}

function ScanStatus({ status, waiting }: { status: StorageAnalysisStatus | null; waiting: boolean }) {
  const progress = status?.progress;
  const terminal = status !== null && !isActiveState(status.state);
  const stateClass = status?.state ?? 'queued';
  return (
    <div class={'storage-analysis-progress' + (terminal ? ' is-terminal' : '')} aria-live="polite">
      <div class="storage-analysis-progress__head">
        <span class={'storage-analysis-state storage-analysis-state--' + stateClass}><Icon name={terminal ? (status?.state === 'failed' ? 'warning' : 'check') : 'activity'} size={18} /></span>
        <div><StatusHeadline status={status} waiting={waiting} /></div>
      </div>
      <span class={'storage-analysis-progress__track' + (terminal ? '' : ' is-indeterminate')} aria-hidden="true"><i style={{ width: terminal ? '100%' : undefined }} /></span>
      <div class="storage-analysis-progress__stats">
        <div><span>Entries</span><strong>{countFormat.format(progress?.entriesScanned ?? 0)}</strong></div>
        <div><span>Files</span><strong>{countFormat.format(progress?.filesScanned ?? 0)}</strong></div>
        <div><span>Folders</span><strong>{countFormat.format(progress?.directoriesScanned ?? 0)}</strong></div>
        <div><span>Apparent bytes</span><strong>{formatBytes(progress?.bytesScanned ?? 0n)}</strong></div>
      </div>
      {!terminal && <div class="storage-analysis-budgets"><span><i style={{ width: String(progress?.entryBudgetPercent ?? 0) + '%' }} /><small>Entry limit {progress?.entryBudgetPercent ?? 0}%</small></span><span><i style={{ width: String(progress?.durationBudgetPercent ?? 0) + '%' }} /><small>Time limit {progress?.durationBudgetPercent ?? 0}%</small></span></div>}
    </div>
  );
}

function IdleScan({
  path,
  busy,
  mode,
  onModeChange,
  onStart,
}: {
  path: string;
  busy: boolean;
  mode: StorageAnalysisMode;
  onModeChange: (mode: StorageAnalysisMode) => void;
  onStart: () => void;
}) {
  return (
    <div class="storage-analysis-idle">
      <div class="storage-analysis-target"><Icon name="folder" size={18} /><div><span>Selected folder</span><code>{path}</code></div></div>
      <div class="storage-analysis-modes" role="radiogroup" aria-label="Scan depth">
        <button type="button" role="radio" aria-checked={mode === 'quick'} class={mode === 'quick' ? 'is-selected' : ''} onClick={() => onModeChange('quick')}><span><Icon name="activity" size={17} /><strong>Quick scan</strong></span><small>Up to 30 seconds and 250,000 entries. Best for normal folders.</small></button>
        <button type="button" role="radio" aria-checked={mode === 'thorough'} class={mode === 'thorough' ? 'is-selected' : ''} onClick={() => onModeChange('thorough')}><span><Icon name="search" size={17} /><strong>Thorough scan</strong></span><small>Up to 10 minutes and 5 million entries. Runs only when you start it.</small></button>
      </div>
      <div class="storage-analysis-guards">
        <div><Icon name="info" size={17} /><strong>Metadata only</strong><span>Names, types, and logical sizes. File contents are never opened.</span></div>
        <div><Icon name="stop" size={17} /><strong>Hard limits</strong><span>{mode === 'thorough' ? '10 minutes, 5 million entries, 128 levels' : '30 seconds, 250,000 entries, 64 levels'}, and bounded result lists.</span></div>
        <div><Icon name="storage" size={17} /><strong>Stays contained</strong><span>No symlink traversal and no crossing onto another filesystem.</span></div>
      </div>
      <p>All eligible entries are considered when a scan completes. Helix retains the largest ranking rows instead of loading millions of rows into the browser; use Files pagination to inspect every directory entry.</p>
      <button class="button button--primary storage-analysis-start" type="button" disabled={busy} onClick={onStart}><Icon name="search" size={16} />{busy ? 'Starting…' : mode === 'thorough' ? 'Start thorough scan' : 'Analyze this folder'}</button>
    </div>
  );
}

export function StorageAnalyzer({
  path,
  csrfToken,
  onClose,
  onNavigate,
  onSessionExpired,
}: StorageAnalyzerProps) {
  const [jobId, setJobId] = useState<string | null>(null);
  const [status, setStatus] = useState<StorageAnalysisStatus | null>(null);
  const [starting, setStarting] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pollCycle, setPollCycle] = useState(0);
  const [mode, setMode] = useState<StorageAnalysisMode>('quick');
  const [trashTarget, setTrashTarget] = useState<StorageAnalysisFile | null>(null);
  const [trashBusy, setTrashBusy] = useState(false);
  const [trashedPaths, setTrashedPaths] = useState<ReadonlySet<string>>(() => new Set());
  const mutation = useRef<AbortController | null>(null);
  const active = jobId !== null && (status === null || isActiveState(status.state));
  const modalRef = useModalFocus(onClose);

  useEffect(() => () => mutation.current?.abort(), []);

  useEffect(() => {
    if (jobId === null || (status !== null && !isActiveState(status.state))) return;
    const controller = new AbortController();
    let timer: number | undefined;
    const poll = async (): Promise<void> => {
      try {
        const next = await getStorageAnalysisStatus(jobId, csrfToken, controller.signal);
        if (controller.signal.aborted) return;
        setStatus((current) => current?.cancelRequested === true && !next.cancelRequested ? current : next);
        setError(null);
        if (isActiveState(next.state)) {
          timer = window.setTimeout(() => void poll(), POLL_INTERVAL_MS);
        }
      } catch (requestError) {
        if (controller.signal.aborted) return;
        if (isSessionError(requestError)) onSessionExpired();
        else setError(describeError(requestError));
      }
    };
    void poll();
    return () => {
      controller.abort();
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [csrfToken, jobId, pollCycle]);

  const begin = async (): Promise<void> => {
    mutation.current?.abort();
    const controller = new AbortController();
    mutation.current = controller;
    setStarting(true);
    setError(null);
    setStatus(null);
    setJobId(null);
    setTrashedPaths(new Set());
    try {
      const dispatch = await startStorageAnalysis(path, csrfToken, mode, controller.signal);
      if (!controller.signal.aborted) setJobId(dispatch.jobId);
    } catch (requestError) {
      if (controller.signal.aborted) return;
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      if (!controller.signal.aborted) setStarting(false);
    }
  };

  const cancel = async (): Promise<void> => {
    if (jobId === null) return;
    mutation.current?.abort();
    const controller = new AbortController();
    mutation.current = controller;
    setCancelling(true);
    setError(null);
    try {
      const next = await cancelStorageAnalysis(jobId, csrfToken, controller.signal);
      if (!controller.signal.aborted) setStatus(next);
    } catch (requestError) {
      if (controller.signal.aborted) return;
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      if (!controller.signal.aborted) setCancelling(false);
    }
  };

  const navigate = (target: string): void => {
    onClose();
    onNavigate(target);
  };

  return (
    <div class="dialog-backdrop storage-analysis-backdrop" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section ref={modalRef} tabIndex={-1} class="dialog storage-analysis-dialog" role="dialog" aria-modal="true" aria-labelledby="storage-analysis-title">
        <header><div><span class="eyebrow">STORAGE</span><h2 id="storage-analysis-title">Folder size analysis <InfoTip text="A user-triggered, read-only metadata scan of the folder currently open in Files." /></h2></div><button class="icon-button" type="button" data-modal-autofocus onClick={onClose} aria-label="Close storage analysis"><Icon name="close" /></button></header>
        <div class="storage-analysis-body">
          <InlineError message={error} />
          {jobId === null && status === null ? (
            <IdleScan path={path} busy={starting} mode={mode} onModeChange={setMode} onStart={() => void begin()} />
          ) : (
            <>
              <div class="storage-analysis-path"><span>Scan root</span><code>{status?.requestedPath ?? path}</code></div>
              <ScanStatus status={status} waiting={status === null} />
              {error !== null && active && <button class="button button--quiet storage-analysis-retry" type="button" onClick={() => setPollCycle((value) => value + 1)}><Icon name="refresh" size={15} />Retry progress check</button>}
              {status?.result !== null && status?.result !== undefined && <StorageAnalysisResults result={status.result} onNavigate={navigate} onTrashRequest={setTrashTarget} hiddenPaths={trashedPaths} />}
              {status?.state === 'failed' && <div class="storage-analysis-failure"><Icon name="warning" size={18} /><span>{status.error ?? 'The storage analysis worker stopped safely.'}</span></div>}
            </>
          )}
        </div>
        <footer class="storage-analysis-footer">
          <span>{active ? 'Closing this window does not cancel the bounded background job.' : trashedPaths.size > 0 ? `${countFormat.format(trashedPaths.size)} item${trashedPaths.size === 1 ? '' : 's'} moved to recoverable trash.` : 'The scan itself never changes files or folders.'}</span>
          <div>
            {active && <button class="button button--danger-quiet" type="button" disabled={cancelling || status?.cancelRequested === true} onClick={() => void cancel()}><Icon name="stop" size={14} />{status?.cancelRequested === true || cancelling ? 'Stopping…' : 'Cancel scan'}</button>}
            {status !== null && !isActiveState(status.state) && <button class="button button--quiet" type="button" onClick={() => void begin()}><Icon name="refresh" size={14} />Scan again</button>}
            <button class="button" type="button" onClick={onClose}>Close</button>
          </div>
        </footer>
      </section>
      {trashTarget !== null && <Dialog title="Move analyzed file to trash?" onClose={() => !trashBusy && setTrashTarget(null)}><div class="dialog-copy"><p><strong>{trashTarget.name}</strong> will move into this drive’s protected <code>.helix-trash</code> folder. This action does not permanently erase it.</p><code class="storage-analysis-trash-path">{trashTarget.path}</code></div><div class="dialog-actions"><button class="button button--quiet" type="button" disabled={trashBusy} onClick={() => setTrashTarget(null)}>Cancel</button><button class="button button--danger" type="button" disabled={trashBusy} onClick={() => { const target = trashTarget; setTrashBusy(true); setError(null); void trashFile(target.path, csrfToken).then(() => { setTrashedPaths((current) => new Set([...current, target.path])); setTrashTarget(null); }).catch((requestError: unknown) => { if (isSessionError(requestError)) onSessionExpired(); else setError(describeError(requestError)); }).finally(() => setTrashBusy(false)); }}><Icon name="trash" size={14} />{trashBusy ? 'Moving…' : 'Move to trash'}</button></div></Dialog>}
    </div>
  );
}

export default StorageAnalyzer;
