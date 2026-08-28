import { describe, expect, it } from 'vitest';
import render from 'preact-render-to-string';
import type { FileEntry } from './control-api';
import { fileTypeLabel, isTextEditable } from './file-manager';
import { FileManagerRoute, loadFileManager } from './file-manager-route';

describe('File manager route loader', () => {
  it('renders an accessible lightweight loading state', () => {
    const markup = render(<FileManagerRoute csrfToken="csrf" onSessionExpired={() => undefined} initialPath="/" />);
    expect(markup).toContain('aria-busy="true"');
    expect(markup).toContain('role="status"');
    expect(markup).toContain('Loading the file manager…');
  });

  it('shares one in-flight import and resolves the real file manager', async () => {
    const first = loadFileManager();
    const second = loadFileManager();
    expect(second).toBe(first);
    await expect(first).resolves.toBeTypeOf('function');
  });
});

describe('File manager entry actions', () => {
  const entry = (name: string, sizeBytes: number, kind: FileEntry['kind'] = 'file'): FileEntry => ({
    name,
    path: `/srv/${name}`,
    kind,
    sizeBytes,
    modifiedUnixMs: null,
    permissions: '0660',
    ownerUid: 1000,
    ownerGid: 1000,
    writable: true,
    restricted: false,
    symlinkTarget: null,
  });

  it('does not offer the text editor for videos or oversized text', () => {
    expect(isTextEditable(entry('clip.mp4', 512))).toBe(false);
    expect(isTextEditable(entry('server.properties', 512))).toBe(true);
    expect(isTextEditable(entry('latest.log', 4 * 1024 * 1024 + 1))).toBe(false);
    expect(isTextEditable(entry('notes.txt', 512, 'directory'))).toBe(false);
  });

  it('labels file types independently from the size column', () => {
    expect(fileTypeLabel(entry('clip.mp4', 512))).toBe('MP4 video');
    expect(fileTypeLabel(entry('archive.zip', 512))).toBe('ZIP archive');
    expect(fileTypeLabel(entry('server.jar', 512))).toBe('Java archive');
    expect(fileTypeLabel(entry('world', 0, 'directory'))).toBe('Folder');
  });
});
