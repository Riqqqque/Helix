import { ApiError, expectArray, expectNumber, expectRecord, expectString, requestJson, type JsonRecord } from './api';

const STRAND_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u;
const MAX_STRANDS = 32;
const MAX_CAPABILITIES = 32;
const MAX_ORIGINS = 8;
const MAX_FILES = 64;

export interface StrandCapability {
  name: string;
  reason: string;
  optional: boolean;
  origins: string[];
}

export interface StrandSummary {
  id: string;
  slug: string;
  name: string;
  version: string;
  description: string;
  license: string;
  publisher: string;
  kind: string;
  enabled: boolean;
  origin: string;
  originDetail: string;
  digestSha256: string;
  uiEntry: string;
  capabilities: StrandCapability[];
  packageBytes: number;
  installedAtUnixMs: number;
  updatedAtUnixMs: number;
  hasPage: boolean;
  hasWidget: boolean;
}

export interface StrandInspect {
  id: string;
  slug: string;
  name: string;
  version: string;
  description: string;
  license: string;
  publisher: string;
  kind: string;
  digestSha256: string;
  uiEntry: string;
  capabilities: StrandCapability[];
  alreadyInstalled: boolean;
  installedVersion: string | null;
  files: Array<{ path: string; bytes: number }>;
}

export type StrandInstallSource =
  | { source: 'upload'; filename: string; bytesBase64: string }
  | { source: 'url'; url: string };

function bool(record: JsonRecord, key: string, context: string): boolean {
  const value = record[key];
  if (typeof value !== 'boolean') throw new ApiError(`${context} returned an invalid ${key} value.`);
  return value;
}

function capability(value: unknown, context: string): StrandCapability {
  const record = expectRecord(value, context);
  const origins = record.origins === undefined
    ? []
    : expectArray(record, 'origins', context, MAX_ORIGINS).map((origin, index) => {
      if (typeof origin !== 'string' || origin.trim().length === 0) {
        throw new ApiError(`${context} origin ${index} is invalid.`);
      }
      return origin;
    });
  return {
    name: expectString(record, 'name', context),
    reason: expectString(record, 'reason', context),
    optional: bool(record, 'optional', context),
    origins,
  };
}

function parseSummary(value: unknown): StrandSummary {
  const record = expectRecord(value, 'Strand');
  const id = expectString(record, 'id', 'Strand');
  if (!STRAND_ID.test(id)) throw new ApiError('Strand returned an invalid id.');
  return {
    id,
    slug: expectString(record, 'slug', 'Strand'),
    name: expectString(record, 'name', 'Strand'),
    version: expectString(record, 'version', 'Strand'),
    description: expectString(record, 'description', 'Strand'),
    license: expectString(record, 'license', 'Strand'),
    publisher: expectString(record, 'publisher', 'Strand'),
    kind: expectString(record, 'kind', 'Strand'),
    enabled: bool(record, 'enabled', 'Strand'),
    origin: expectString(record, 'origin', 'Strand'),
    originDetail: expectString(record, 'originDetail', 'Strand'),
    digestSha256: expectString(record, 'digestSha256', 'Strand'),
    uiEntry: expectString(record, 'uiEntry', 'Strand'),
    capabilities: expectArray(record, 'capabilities', 'Strand', MAX_CAPABILITIES).map((item, index) => capability(item, `Strand capability ${index}`)),
    packageBytes: expectNumber(record, 'packageBytes', 'Strand', { integer: true, minimum: 32 }),
    installedAtUnixMs: expectNumber(record, 'installedAtUnixMs', 'Strand', { integer: true, minimum: 0 }),
    updatedAtUnixMs: expectNumber(record, 'updatedAtUnixMs', 'Strand', { integer: true, minimum: 0 }),
    hasPage: bool(record, 'hasPage', 'Strand'),
    hasWidget: bool(record, 'hasWidget', 'Strand'),
  };
}

function parseInspect(value: unknown): StrandInspect {
  const record = expectRecord(value, 'Strand inspect');
  const files = expectArray(record, 'files', 'Strand inspect', MAX_FILES).map((item, index) => {
    const file = expectRecord(item, `Strand inspect file ${index}`);
    return {
      path: expectString(file, 'path', `Strand inspect file ${index}`),
      bytes: expectNumber(file, 'bytes', `Strand inspect file ${index}`, { integer: true, minimum: 0 }),
    };
  });
  return {
    id: expectString(record, 'id', 'Strand inspect'),
    slug: expectString(record, 'slug', 'Strand inspect'),
    name: expectString(record, 'name', 'Strand inspect'),
    version: expectString(record, 'version', 'Strand inspect'),
    description: expectString(record, 'description', 'Strand inspect'),
    license: expectString(record, 'license', 'Strand inspect'),
    publisher: expectString(record, 'publisher', 'Strand inspect'),
    kind: expectString(record, 'kind', 'Strand inspect'),
    digestSha256: expectString(record, 'digestSha256', 'Strand inspect'),
    uiEntry: expectString(record, 'uiEntry', 'Strand inspect'),
    capabilities: expectArray(record, 'capabilities', 'Strand inspect', MAX_CAPABILITIES).map((item, index) => capability(item, `Strand inspect capability ${index}`)),
    alreadyInstalled: bool(record, 'alreadyInstalled', 'Strand inspect'),
    installedVersion: record.installedVersion === null || record.installedVersion === undefined
      ? null
      : expectString(record, 'installedVersion', 'Strand inspect'),
    files,
  };
}

function parseList(value: unknown): StrandSummary[] {
  const record = expectRecord(value, 'Strands');
  return expectArray(record, 'strands', 'Strands', MAX_STRANDS).map(parseSummary);
}

export function parseStrandInventory(value: unknown): StrandSummary[] {
  return parseList(value);
}

export function listStrands(csrfToken: string, signal?: AbortSignal): Promise<StrandSummary[]> {
  return requestJson('/api/v1/strands', parseList, { csrfToken, signal });
}

export function inspectStrand(csrfToken: string, source: StrandInstallSource): Promise<StrandInspect> {
  return requestJson('/api/v1/strands/inspect', parseInspect, {
    method: 'POST',
    csrfToken,
    body: source,
    timeoutMs: 30_000,
  });
}

export function installStrand(csrfToken: string, source: StrandInstallSource): Promise<StrandSummary> {
  return requestJson('/api/v1/strands', parseSummary, {
    method: 'POST',
    csrfToken,
    body: source,
    timeoutMs: 30_000,
  });
}

export function setStrandEnabled(csrfToken: string, id: string, enabled: boolean): Promise<StrandSummary> {
  return requestJson(`/api/v1/strands/${id}`, parseSummary, {
    method: 'PUT',
    csrfToken,
    body: { enabled },
  });
}

export function deleteStrand(csrfToken: string, id: string): Promise<void> {
  return requestJson(`/api/v1/strands/${id}`, (value) => {
    const record = expectRecord(value, 'Strand delete');
    if (record.deleted !== true) throw new ApiError('Helix could not remove that Strand.');
  }, {
    method: 'DELETE',
    csrfToken,
  });
}

export async function downloadStrandPackage(csrfToken: string, id: string, filename: string): Promise<void> {
  const response = await fetch(`/api/v1/strands/${id}/package`, {
    headers: { 'X-Helix-CSRF': csrfToken },
    credentials: 'same-origin',
    cache: 'no-store',
  });
  if (!response.ok) throw new ApiError('Helix could not export that Strand zip.');
  const blob = await response.blob();
  const href = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = href;
  link.download = filename;
  link.click();
  URL.revokeObjectURL(href);
}

export function strandHostCall(
  csrfToken: string,
  id: string,
  method: string,
  params: Record<string, unknown>,
): Promise<unknown> {
  return requestJson(`/api/v1/strands/${id}/host`, (value) => value, {
    method: 'POST',
    csrfToken,
    body: { method, params },
    timeoutMs: 35_000,
  });
}

export function strandFileUrl(id: string, uiEntry: string): string {
  return `/api/v1/strands/${id}/files/${uiEntry.split('/').map(encodeURIComponent).join('/')}`;
}

export function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = typeof reader.result === 'string' ? reader.result : '';
      const marker = 'base64,';
      const index = result.indexOf(marker);
      resolve(index >= 0 ? result.slice(index + marker.length) : result);
    };
    reader.onerror = () => reject(new ApiError('The Strand zip could not be read.'));
    reader.readAsDataURL(file);
  });
}
