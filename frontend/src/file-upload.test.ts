import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  bytesToBase64,
  droppedTransferFiles,
  isSafeUploadName,
  MAX_CUSTOM_JAR_UPLOAD_BYTES,
  MAX_STORAGE_UPLOAD_BYTES,
  maxUploadBytes,
  uploadBaseName,
  uploadHostFile,
} from './file-upload';

afterEach(() => vi.unstubAllGlobals());

describe('file uploads', () => {
  it('keeps only the basename and rejects unsafe names', () => {
    expect(uploadBaseName('C:\\\\mods\\\\worldedit.jar')).toBe('worldedit.jar');
    expect(uploadBaseName('/plugins/foo.jar')).toBe('foo.jar');
    expect(isSafeUploadName('server.jar')).toBe(true);
    expect(isSafeUploadName('.hidden.jar')).toBe(false);
    expect(isSafeUploadName('../escape.jar')).toBe(false);
    expect(isSafeUploadName('')).toBe(false);
  });

  it('rejects dropped folders before any upload starts', () => {
    const data = {
      items: [{ webkitGetAsEntry: () => ({ isDirectory: true }) }],
      files: [],
    } as unknown as DataTransfer;
    expect(() => droppedTransferFiles(data)).toThrow(/folders/i);
    expect(droppedTransferFiles(null)).toEqual([]);
  });

  it('encodes chunks and keeps purpose-specific size caps', () => {
    expect(bytesToBase64(new Uint8Array([80, 75, 3, 4]))).toBe('UEsDBA==');
    expect(maxUploadBytes('storage')).toBe(MAX_STORAGE_UPLOAD_BYTES);
    expect(maxUploadBytes('custom_jar')).toBe(MAX_CUSTOM_JAR_UPLOAD_BYTES);
  });

  it('streams a custom JAR through begin, chunk, and finish', async () => {
    const bytes = new Uint8Array(16 * 1024);
    bytes.set([0x50, 0x4b, 0x03, 0x04]);
    const file = new File([bytes], 'paper.jar', { type: 'application/java-archive' });
    const fetchMock = vi.fn().mockImplementation(async (path: string) => {
      if (String(path).endsWith('/begin')) {
        return new Response(JSON.stringify({
          upload_id: 'upload-1',
          expected_size: file.size,
          max_chunk_bytes: 2 * 1024 * 1024,
          purpose: 'custom_jar',
        }), { status: 200 });
      }
      if (String(path).endsWith('/chunk')) {
        return new Response(JSON.stringify({
          upload_id: 'upload-1',
          bytes_written: file.size,
          expected_size: file.size,
        }), { status: 200 });
      }
      if (String(path).endsWith('/finish')) {
        return new Response(JSON.stringify({ path: '/var/lib/helix/imports/paper.jar' }), { status: 200 });
      }
      return new Response('{}', { status: 200 });
    });
    vi.stubGlobal('fetch', fetchMock);

    await expect(uploadHostFile({
      file,
      purpose: 'custom_jar',
      csrfToken: 'csrf',
    })).resolves.toEqual({ path: '/var/lib/helix/imports/paper.jar' });

    expect(fetchMock.mock.calls.map((call) => String(call[0]))).toEqual([
      '/api/v1/files/upload/begin',
      '/api/v1/files/upload/chunk',
      '/api/v1/files/upload/finish',
    ]);
    const [, beginRequest] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(JSON.parse(String(beginRequest.body))).toEqual({
      target: { kind: 'custom_jar' },
      name: 'paper.jar',
      expected_size: file.size,
    });
  });

  it('does not start a custom JAR upload for a non-jar name', async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    await expect(uploadHostFile({
      file: new File(['nope'], 'notes.txt'),
      purpose: 'custom_jar',
      csrfToken: 'csrf',
    })).rejects.toThrow(/\.jar/i);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
