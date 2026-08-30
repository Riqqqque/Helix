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
  let key = '';
  for (const character of value.normalize('NFKC')) {
    const mapped = mapCurseforgeKeyCharacter(character);
    if (mapped !== null) key += mapped;
  }
  key = key.trim();
  for (let pass = 0; pass < 3; pass += 1) {
    const unquoted = stripWrappingQuotes(key);
    if (unquoted === key) break;
    key = unquoted;
  }
  if (key.includes('$$2')) key = key.replaceAll('$$', '$');
  return extractCurseforgeConsoleKey(key) ?? [...key].filter((character) => {
    const code = character.codePointAt(0) ?? 0;
    return code >= 0x21 && code <= 0x7e;
  }).join('');
}

function mapCurseforgeKeyCharacter(character: string): string | null {
  const code = character.codePointAt(0) ?? 0;
  if (code <= 0x1f || code === 0x7f) return null;
  if (
    code === 0xad || code === 0x34f || code === 0x61c || code === 0x180e
    || (code >= 0x200b && code <= 0x200f) || (code >= 0x202a && code <= 0x202e)
    || (code >= 0x2060 && code <= 0x2064) || (code >= 0x2066 && code <= 0x206f)
    || code === 0xfeff || (code >= 0xfff9 && code <= 0xfffb)
  ) {
    return null;
  }
  if (code === 0xa0 || code === 0x202f || (code >= 0x2007 && code <= 0x200a)) return ' ';
  if (code === 0x2018 || code === 0x2019 || code === 0x201a || code === 0x201b) return "'";
  if (code === 0x201c || code === 0x201d || code === 0x201e || code === 0x201f) return '"';
  if ((code >= 0x2010 && code <= 0x2015) || code === 0x2212) return '-';
  return character;
}

function stripWrappingQuotes(value: string): string {
  const key = value.trim();
  if (
    (key.startsWith('"') && key.endsWith('"'))
    || (key.startsWith("'") && key.endsWith("'"))
    || (key.startsWith('\u201c') && key.endsWith('\u201d'))
    || (key.startsWith('\u2018') && key.endsWith('\u2019'))
  ) {
    return key.slice(1, -1).trim();
  }
  return key;
}

function extractCurseforgeConsoleKey(value: string): string | null {
  const start = value.indexOf('$2');
  if (start < 0) return null;
  let token = '';
  for (const character of value.slice(start)) {
    if (character === '$' || character === '/' || character === '.' || /[A-Za-z0-9]/u.test(character)) {
      token += character;
      if (token.length >= 256) break;
    } else {
      break;
    }
  }
  return token.length >= 24 ? token : null;
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
