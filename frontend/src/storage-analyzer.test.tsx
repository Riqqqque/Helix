import render from 'preact-render-to-string';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  sortStorageAnalysisFiles,
  sortStorageAnalysisFolders,
  StorageAnalysisResults,
  StorageAnalyzer,
  storageResultNavigationPath,
} from './storage-analyzer';
import type {
  StorageAnalysisFile,
  StorageAnalysisFolder,
  StorageAnalysisResult,
} from './storage-analysis-api';

const files: StorageAnalysisFile[] = [
  { path: '/srv/media/small.bin', name: 'small.bin', type: 'file', bytes: 12n },
  { path: '/srv/media/large.bin', name: 'large.bin', type: 'file', bytes: 9_007_199_254_740_999n },
  { path: '/srv/media/alpha.bin', name: 'alpha.bin', type: 'file', bytes: 512n },
];
const folders: StorageAnalysisFolder[] = [
  {
    path: '/srv/media/direct',
    name: 'direct',
    type: 'directory',
    immediateBytes: 10_000n,
    recursiveBytes: 11_000n,
    immediateComplete: true,
    recursiveComplete: true,
  },
  {
    path: '/srv/media/tree',
    name: 'tree',
    type: 'directory',
    immediateBytes: 100n,
    recursiveBytes: 50_000n,
    immediateComplete: true,
    recursiveComplete: false,
  },
];
const result: StorageAnalysisResult = {
  rootPath: '/srv/media',
  apparentBytesScanned: 61_000n,
  truncated: true,
  stopReason: 'entry_limit',
  errors: {
    total: 2,
    permissionDenied: 1,
    filesystemRaces: 1,
    metadataFailures: 0,
    sizeOverflows: 0,
  },
  skipped: {
    restrictedPaths: 1,
    symbolicLinks: 2,
    otherFilesystems: 1,
    specialFiles: 0,
    depthLimitedDirectories: 0,
    unrepresentableNames: 0,
  },
  fileResultsOmitted: 7,
  recursiveFolderResultsOmitted: 3,
  immediateFolderResultsOmitted: 3,
  responseResultsOmitted: 1,
  largestFiles: files,
  largestFoldersByRecursiveBytes: folders,
  largestFoldersByImmediateBytes: folders,
  limits: {
    mode: 'quick',
    maxDurationMs: 30_000,
    maxEntries: 250_000,
    maxDepth: 64,
    maxResultsPerList: 128,
    maxResponseBytes: 1_048_576,
    stayOnTargetFilesystem: true,
    followsSymbolicLinks: false,
  },
};

afterEach(() => vi.unstubAllGlobals());

describe('Storage analyzer', () => {
  it('does not start a scan when the analyzer is opened or rendered', () => {
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);

    const markup = render(
      <StorageAnalyzer
        path="/srv/media"
        csrfToken="csrf"
        onClose={() => undefined}
        onNavigate={() => undefined}
        onSessionExpired={() => undefined}
      />,
    );

    expect(markup).toContain('Analyze this folder');
    expect(markup).toContain('tabindex="-1"');
    expect(markup).toContain('Runs only when you start it');
    expect(markup).toContain('Thorough scan');
    expect(markup).toContain('File contents are never opened');
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('sorts exact bigint sizes without mutating backend order', () => {
    expect(sortStorageAnalysisFiles(files, 'size-desc').map((file) => file.name)).toEqual([
      'large.bin',
      'alpha.bin',
      'small.bin',
    ]);
    expect(sortStorageAnalysisFiles(files, 'name-asc').map((file) => file.name)).toEqual([
      'alpha.bin',
      'large.bin',
      'small.bin',
    ]);
    expect(files.map((file) => file.name)).toEqual(['small.bin', 'large.bin', 'alpha.bin']);
    expect(sortStorageAnalysisFolders(folders, 'recursive', 'size-desc')[0]?.name).toBe('tree');
    expect(sortStorageAnalysisFolders(folders, 'immediate', 'size-desc')[0]?.name).toBe('direct');
  });

  it('navigates folders directly and files through their containing folder', () => {
    expect(storageResultNavigationPath(files[0]!)).toBe('/srv/media');
    expect(storageResultNavigationPath({ ...files[0]!, path: '/root.bin' })).toBe('/');
    expect(storageResultNavigationPath(folders[0]!)).toBe('/srv/media/direct');
  });

  it('renders partial, skipped, omitted, and incomplete coverage honestly', () => {
    const markup = render(<StorageAnalysisResults result={result} onNavigate={() => undefined} onTrashRequest={() => undefined} />);

    expect(markup).toContain('Partial scan coverage');
    expect(markup).toContain('250,000-entry safety limit');
    expect(markup).toContain('Filesystem errors');
    expect(markup).toContain('Changed during scan');
    expect(markup).toContain('entries were excluded');
    expect(markup).toContain('largest ranking rows are retained');
    expect(markup).toContain('Apparent bytes analyzed');
    expect(markup).toContain('large.bin');
    expect(markup).toContain('Show in files');
    expect(markup).toContain('Move to trash');
    expect(markup).toContain('Logical file length from metadata');
  });

  it('does not call a completed top-N ranking a partial filesystem scan', () => {
    const complete = {
      ...result,
      truncated: false,
      stopReason: null,
      errors: { total: 0, permissionDenied: 0, filesystemRaces: 0, metadataFailures: 0, sizeOverflows: 0 },
      skipped: { ...result.skipped, restrictedPaths: 0, symbolicLinks: 0, otherFilesystems: 0 },
    } satisfies StorageAnalysisResult;
    const markup = render(<StorageAnalysisResults result={complete} onNavigate={() => undefined} />);

    expect(markup).toContain('Full scan coverage');
    expect(markup).toContain('Every eligible entry');
    expect(markup).not.toContain('Partial scan coverage');
  });
});
