import { ApiError, expectRecord, expectString, requestJson } from './api';

export const CURSEFORGE_CONSOLE_URL = 'https://console.curseforge.com/';
export const CURSEFORGE_KEY_REQUIRED_HINT = 'Settings → Catalogs';

export interface CurseforgeKeyStatus {
  configured: boolean;
  catalog: 'api.curseforge.com';
}

export function isCurseforgeKeyRequired(message: string): boolean {
  return message.includes(CURSEFORGE_KEY_REQUIRED_HINT) || message.includes('console.curseforge.com');
}

function parseCurseforgeKeyStatus(value: unknown): CurseforgeKeyStatus {
  const root = expectRecord(value, 'CurseForge catalog');
  if (root.schema_version !== 1) throw new ApiError('CurseForge catalog returned an unsupported schema.');
  if (typeof root.configured !== 'boolean') throw new ApiError('CurseForge catalog returned an invalid configured value.');
  const catalog = expectString(root, 'catalog', 'CurseForge catalog');
  if (catalog !== 'api.curseforge.com') throw new ApiError('CurseForge catalog returned an unexpected host.');
  return { configured: root.configured, catalog };
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
