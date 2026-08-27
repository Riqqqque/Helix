import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  cancelStorageAnalysis,
  getStorageAnalysisStatus,
  parseStorageAnalysisStatus,
  startStorageAnalysis,
} from './storage-analysis-api';

const csrf = 'SSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSS';
const jobId = '019d1234-5678-4abc-8def-123456789abc';

const limits = {
  mode: 'quick',
  max_duration_ms: 30_000,
  max_entries: 250_000,
  max_depth: 64,
  max_results_per_list: 128,
  max_response_bytes: 1_048_576,
  stay_on_target_filesystem: true,
  follows_symbolic_links: false,
};

const progress = {
  percent: null,
  entry_budget_percent: 1,
  duration_budget_percent: 4,
  entries_scanned: 25,
  files_scanned: 20,
  directories_scanned: 6,
  bytes_scanned: '18446744073709551615',
};

const result = {
  root_path: '/srv/media',
  apparent_bytes_scanned: '18446744073709551615',
  truncated: true,
  stop_reason: 'entry_limit',
  errors: {
    total: 3,
    permission_denied: 1,
    filesystem_races: 1,
    metadata_failures: 1,
    size_overflows: 0,
  },
  skipped: {
    restricted_paths: 2,
    symbolic_links: 1,
    other_filesystems: 1,
    special_files: 0,
    depth_limited_directories: 0,
    unrepresentable_names: 0,
  },
  file_results_omitted: 4,
  recursive_folder_results_omitted: 2,
  immediate_folder_results_omitted: 2,
  response_results_omitted: 1,
  largest_files: [{
    path: '/srv/media/movie.mkv',
    name: 'movie.mkv',
    type: 'file',
    bytes: '10995116277760',
  }],
  largest_folders_by_recursive_bytes: [{
    path: '/srv/media/shows',
    name: 'shows',
    type: 'directory',
    immediate_bytes: '1024',
    recursive_bytes: '21990232555520',
    immediate_complete: true,
    recursive_complete: false,
  }],
  largest_folders_by_immediate_bytes: [{
    path: '/srv/media/movies',
    name: 'movies',
    type: 'directory',
    immediate_bytes: '10995116277760',
    recursive_bytes: '10995116277760',
    immediate_complete: true,
    recursive_complete: true,
  }],
  limits,
};

function runningStatus(): Record<string, unknown> {
  return {
    job_id: jobId,
    requested_path: '/srv/media',
    state: 'running',
    progress,
    created_unix_ms: 1_800_000_000_000,
    started_unix_ms: 1_800_000_000_010,
    finished_unix_ms: null,
    cancel_requested: false,
    result: null,
    error: null,
  };
}

function terminalStatus(
  state: 'complete' | 'cancelled' = 'complete',
): Record<string, unknown> {
  const terminalResult = {
    ...result,
    stop_reason: state === 'cancelled' ? 'cancelled' : result.stop_reason,
  };
  return {
    ...runningStatus(),
    state,
    progress: { ...progress, percent: 100 },
    finished_unix_ms: 1_800_000_030_000,
    cancel_requested: state === 'cancelled',
    result: terminalResult,
  };
}

afterEach(() => vi.unstubAllGlobals());

describe('storage analysis API contract', () => {
  it('preserves exact byte counts and every partial-result signal', () => {
    const parsed = parseStorageAnalysisStatus(terminalStatus());

    expect(parsed.progress.bytesScanned).toBe(18_446_744_073_709_551_615n);
    expect(parsed.result).toMatchObject({
      truncated: true,
      stopReason: 'entry_limit',
      fileResultsOmitted: 4,
      responseResultsOmitted: 1,
    });
    expect(parsed.result?.largestFiles[0]).toMatchObject({
      name: 'movie.mkv',
      bytes: 10_995_116_277_760n,
    });
    expect(parsed.result?.largestFoldersByRecursiveBytes[0]).toMatchObject({
      recursiveBytes: 21_990_232_555_520n,
      recursiveComplete: false,
    });
  });

  it('rejects unsafe paths, invalid decimal bytes, and dishonest totals', () => {
    const unsafePath = structuredClone(terminalStatus());
    const unsafeResult = unsafePath.result as typeof result;
    unsafeResult.largest_files[0]!.path = '/srv/media/../shadow';
    expect(() => parseStorageAnalysisStatus(unsafePath)).toThrow(/invalid path/i);

    const invalidBytes = structuredClone(terminalStatus());
    const byteResult = invalidBytes.result as typeof result;
    byteResult.largest_files[0]!.bytes = '01024';
    expect(() => parseStorageAnalysisStatus(invalidBytes)).toThrow(/invalid bytes/i);

    const dishonestErrors = structuredClone(terminalStatus());
    const errorResult = dishonestErrors.result as typeof result;
    errorResult.errors.total = 2;
    expect(() => parseStorageAnalysisStatus(dishonestErrors)).toThrow(/invalid total/i);

    const dishonestCompleteness = structuredClone(terminalStatus());
    const completeResult = dishonestCompleteness.result as typeof result;
    completeResult.truncated = false;
    expect(() => parseStorageAnalysisStatus(dishonestCompleteness)).toThrow(/marked partial/i);

    const mismatchedName = structuredClone(terminalStatus());
    const nameResult = mismatchedName.result as typeof result;
    nameResult.largest_files[0]!.name = 'different.mkv';
    expect(() => parseStorageAnalysisStatus(mismatchedName)).toThrow(/invalid name/i);
  });

  it('accepts the backend root-folder representation without weakening path checks', () => {
    const rootStatus = structuredClone(terminalStatus());
    rootStatus.requested_path = '/';
    const rootResult = rootStatus.result as typeof result;
    rootResult.root_path = '/';
    rootResult.largest_folders_by_recursive_bytes[0]!.path = '/';
    rootResult.largest_folders_by_recursive_bytes[0]!.name = '/';
    rootResult.largest_folders_by_immediate_bytes[0]!.path = '/';
    rootResult.largest_folders_by_immediate_bytes[0]!.name = '/';

    expect(parseStorageAnalysisStatus(rootStatus).result?.rootPath).toBe('/');
  });

  it('accepts omitted top-ranking rows without calling scan coverage partial', () => {
    const complete = structuredClone(terminalStatus());
    const completeResult = complete.result as typeof result;
    completeResult.truncated = false;
    (completeResult as Record<string, unknown>).stop_reason = null;
    completeResult.errors = {
      total: 0,
      permission_denied: 0,
      filesystem_races: 0,
      metadata_failures: 0,
      size_overflows: 0,
    };
    completeResult.skipped.depth_limited_directories = 0;

    expect(parseStorageAnalysisStatus(complete).result).toMatchObject({
      truncated: false,
      fileResultsOmitted: 4,
    });
  });

  it('rejects contradictory job states and responses beyond the row bound', () => {
    expect(() => parseStorageAnalysisStatus({
      ...runningStatus(),
      state: 'complete',
      finished_unix_ms: 1_800_000_030_000,
    })).toThrow(/inconsistent terminal state/i);

    const tooManyRows = structuredClone(terminalStatus());
    const rowResult = tooManyRows.result as typeof result;
    rowResult.largest_files = Array.from({ length: 129 }, (_, index) => ({
      path: `/srv/media/${index}.bin`,
      name: `${index}.bin`,
      type: 'file' as const,
      bytes: String(index),
    }));
    expect(() => parseStorageAnalysisStatus(tooManyRows)).toThrow(/largest_files/i);
  });

  it('uses the exact start, status, and cancellation requests', async () => {
    const responses = [
      { job_id: jobId, state: 'queued' },
      runningStatus(),
      terminalStatus('cancelled'),
    ];
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(new Response(
      JSON.stringify(responses.shift()),
      { status: 200, headers: { 'Content-Type': 'application/json' } },
    )));
    vi.stubGlobal('fetch', fetchMock);

    await startStorageAnalysis('/srv/media', csrf, 'thorough');
    await getStorageAnalysisStatus(jobId, csrf);
    await cancelStorageAnalysis(jobId, csrf);

    expect(fetchMock.mock.calls.map((call) => call[0])).toEqual([
      '/api/v1/storage/analysis',
      `/api/v1/storage/analysis/${jobId}`,
      `/api/v1/storage/analysis/${jobId}`,
    ]);
    const start = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(start.method).toBe('POST');
    expect(JSON.parse(String(start.body))).toEqual({ path: '/srv/media', mode: 'thorough' });
    const cancel = fetchMock.mock.calls[2]?.[1] as RequestInit;
    expect(cancel.method).toBe('DELETE');
    expect(JSON.parse(String(cancel.body))).toEqual({});
    expect(new Headers(cancel.headers).get('X-Helix-CSRF')).toBe(csrf);
  });

  it('rejects invalid client input before opening a network request', () => {
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);

    expect(() => startStorageAnalysis('../etc', csrf)).toThrow(/path is invalid/i);
    expect(() => startStorageAnalysis('/srv/media', csrf, 'invalid' as 'quick')).toThrow(/mode is invalid/i);
    expect(() => getStorageAnalysisStatus('not-a-job', csrf)).toThrow(/job ID/i);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
