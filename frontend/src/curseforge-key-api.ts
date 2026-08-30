import { ApiError, expectRecord, expectString, requestJson } from './api';

export const CURSEFORGE_CONSOLE_URL = 'https://console.curseforge.com/';
export const CURSEFORGE_KEY_REQUIRED_HINT = 'Settings → Catalogs';

export type CurseforgeProbe = 'ok' | 'cdn_blocked' | 'unreachable';

export interface CurseforgeKeyStatus {
  configured: boolean;
  catalog: 'api.curseforge.com';
  probe?: CurseforgeProbe;
}

export function isCurseforgeKeyRequired(message: string): boolean {
  return message.includes(CURSEFORGE_KEY_REQUIRED_HINT) || message.includes('console.curseforge.com');
}

export function normalizeCurseforgeApiKey(value: string): string {
  let key = value.replace(/[\u00AD\u200B-\u200D\u2060\uFEFF]/gu, '').replace(/\u00A0/gu, ' ').trim();
  if (
    (key.startsWith('"') && key.endsWith('"'))
    || (key.startsWith("'") && key.endsWith("'"))
    || (key.startsWith('\u201c') && key.endsWith('\u201d'))
    || (key.startsWith('\u2018') && key.endsWith('\u2019'))
  ) {
    key = key.slice(1, -1).trim();
  }
  if (key.startsWith('$$2')) key = key.replaceAll('$$', '$');
  return key.replace(/\s+/gu, '');
}

function parseCurseforgeKeyStatus(value: unknown): CurseforgeKeyStatus {
  const root = expectRecord(value, 'CurseForge catalog');
  if (root.schema_version !== 1) throw new ApiError('CurseForge catalog returned an unsupported schema.');
  if (typeof root.configured !== 'boolean') throw new ApiError('CurseForge catalog returned an invalid configured value.');
  const catalog = expectString(root, 'catalog', 'CurseForge catalog');
  if (catalog !== 'api.curseforge.com') throw new ApiError('CurseForge catalog returned an unexpected host.');
  const status: CurseforgeKeyStatus = { configured: root.configured, catalog };
  if (root.probe !== undefined) {
    if (root.probe !== 'ok' && root.probe !== 'cdn_blocked' && root.probe !== 'unreachable') {
      throw new ApiError('CurseForge catalog returned an invalid probe.');
    }
    status.probe = root.probe;
  }
  return status;
}

export function getCurseforgeKeyStatus(csrfToken: string, signal?: AbortSignal): Promise<CurseforgeKeyStatus> {
  return requestJson('/api/v1/marketplace/curseforge/key', parseCurseforgeKeyStatus, { csrfToken, signal, timeoutMs: 20_000 });
}

export function setCurseforgeApiKey(key: string, csrfToken: string): Promise<CurseforgeKeyStatus> {
  return requestJson('/api/v1/marketplace/curseforge/key', parseCurseforgeKeyStatus, {
    method: 'PUT',
    body: { key },
    csrfToken,
    timeoutMs: 30_000,
  });
}

export function clearCurseforgeApiKey(csrfToken: string): Promise<CurseforgeKeyStatus> {
  return requestJson('/api/v1/marketplace/curseforge/key', parseCurseforgeKeyStatus, {
    method: 'DELETE',
    body: {},
    csrfToken,
    timeoutMs: 20_000,
  });
}
