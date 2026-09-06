import { ApiError, expectNumber, expectRecord, expectString, requestJson } from './api';

export const MAX_STORAGE_UPLOAD_BYTES = 256 * 1024 * 1024;
export const MAX_CUSTOM_JAR_UPLOAD_BYTES = 768 * 1024 * 1024;
export const DEFAULT_UPLOAD_CHUNK_BYTES = 2 * 1024 * 1024;

export type FileUploadPurpose = 'storage' | 'custom_jar';

export interface FileUploadStart {
  uploadId: string;
  expectedSize: number;
  maxChunkBytes: number;
  purpose: FileUploadPurpose;
}

function ignoreResponse(value: unknown): void {
  void value;
}

export function uploadBaseName(name: string): string {
  const normalized = name.replaceAll('\\', '/');
  const parts = normalized.split('/');
  return (parts[parts.length - 1] ?? '').trim();
}

export function isSafeUploadName(name: string): boolean {
  const base = uploadBaseName(name);
  return (
    base === name &&
    base.length > 0 &&
    base.length <= 255 &&
    base !== '.' &&
    base !== '..' &&
    !base.startsWith('.') &&
    !base.includes('/') &&
    !base.includes('\\') &&
    !base.includes('\0')
  );
}

export function maxUploadBytes(purpose: FileUploadPurpose): number {
  return purpose === 'custom_jar' ? MAX_CUSTOM_JAR_UPLOAD_BYTES : MAX_STORAGE_UPLOAD_BYTES;
}

export function droppedTransferFiles(data: DataTransfer | null): File[] {
  if (data === null) return [];
  const items = [...data.items];
  for (const item of items) {
    const entry = item.webkitGetAsEntry?.() ?? null;
    if (entry?.isDirectory === true) {
      throw new Error('Folders cannot be dropped here.');
    }
  }
  return [...data.files];
}

export function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  const step = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += step) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + step));
  }
  return btoa(binary);
}

function parseUploadStart(value: unknown): FileUploadStart {
  const root = expectRecord(value, 'file upload');
  const purpose = expectString(root, 'purpose', 'file upload');
  if (purpose !== 'storage' && purpose !== 'custom_jar') {
    throw new ApiError('File upload returned an invalid purpose.');
  }
  return {
    uploadId: expectString(root, 'upload_id', 'file upload'),
    expectedSize: expectNumber(root, 'expected_size', 'file upload', { integer: true, minimum: 1 }),
    maxChunkBytes: expectNumber(root, 'max_chunk_bytes', 'file upload', {
      integer: true,
      minimum: 1,
    }),
    purpose,
  };
}

function parseUploadedPath(value: unknown): { path: string } {
  const root = expectRecord(value, 'uploaded file');
  return { path: expectString(root, 'path', 'uploaded file') };
}

async function beginUpload(
  purpose: FileUploadPurpose,
  name: string,
  expectedSize: number,
  csrfToken: string,
  parent?: string,
): Promise<FileUploadStart> {
  return requestJson('/api/v1/files/upload/begin', parseUploadStart, {
    method: 'POST',
    body:
      purpose === 'custom_jar'
        ? { target: { kind: 'custom_jar' }, name, expected_size: expectedSize }
        : {
            target: { kind: 'directory', parent },
            name,
            expected_size: expectedSize,
          },
    csrfToken,
    timeoutMs: 20_000,
  });
}

async function abortUpload(
  uploadId: string,
  purpose: FileUploadPurpose,
  csrfToken: string,
): Promise<void> {
  await requestJson('/api/v1/files/upload/abort', ignoreResponse, {
    method: 'POST',
    body: { upload_id: uploadId, purpose },
    csrfToken,
    timeoutMs: 20_000,
  });
}

export async function uploadHostFile(input: {
  file: File;
  purpose: FileUploadPurpose;
  csrfToken: string;
  parent?: string;
  onProgress?: (percent: number) => void;
  signal?: AbortSignal;
}): Promise<{ path: string }> {
  const name = uploadBaseName(input.file.name);
  if (!isSafeUploadName(name)) {
    throw new Error('That file name is not allowed.');
  }
  if (input.purpose === 'custom_jar' && !/\.jar$/iu.test(name)) {
    throw new Error('Drop a .jar file.');
  }
  const maximum = maxUploadBytes(input.purpose);
  if (input.file.size === 0 || input.file.size > maximum) {
    throw new Error(
      input.purpose === 'custom_jar'
        ? 'Custom server JARs must be between 16 KiB and 768 MiB.'
        : 'Uploaded files must be between 1 byte and 256 MiB.',
    );
  }
  if (input.purpose === 'custom_jar' && input.file.size < 16 * 1024) {
    throw new Error('Custom server JARs must be between 16 KiB and 768 MiB.');
  }
  if (input.purpose === 'storage' && (input.parent === undefined || input.parent.length === 0)) {
    throw new Error('Choose a folder before uploading.');
  }

  const started = await beginUpload(
    input.purpose,
    name,
    input.file.size,
    input.csrfToken,
    input.parent,
  );
  const chunkSize = Math.min(
    Math.max(1, started.maxChunkBytes),
    DEFAULT_UPLOAD_CHUNK_BYTES,
  );
  try {
    let offset = 0;
    while (offset < input.file.size) {
      if (input.signal?.aborted === true) throw new Error('Upload cancelled.');
      const end = Math.min(offset + chunkSize, input.file.size);
      const bytes = new Uint8Array(await input.file.slice(offset, end).arrayBuffer());
      await requestJson('/api/v1/files/upload/chunk', ignoreResponse, {
        method: 'POST',
        body: {
          upload_id: started.uploadId,
          purpose: input.purpose,
          offset,
          data_base64: bytesToBase64(bytes),
        },
        csrfToken: input.csrfToken,
        timeoutMs: 45_000,
        signal: input.signal,
      });
      offset = end;
      input.onProgress?.(Math.round((offset / input.file.size) * 100));
    }
    return await requestJson('/api/v1/files/upload/finish', parseUploadedPath, {
      method: 'POST',
      body: { upload_id: started.uploadId, purpose: input.purpose },
      csrfToken: input.csrfToken,
      timeoutMs: 30_000,
      signal: input.signal,
    });
  } catch (error) {
    try {
      await abortUpload(started.uploadId, input.purpose, input.csrfToken);
    } catch {
      // The original upload error is the one to show.
    }
    throw error;
  }
}
