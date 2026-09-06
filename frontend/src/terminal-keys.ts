export const TERMINAL_FONT_SIZE_MIN = 12;
export const TERMINAL_FONT_SIZE_MAX = 22;
export const TERMINAL_FONT_SIZE_DEFAULT = 14;
export const TERMINAL_FONT_SIZE_STORAGE_KEY = 'helix.terminal.font-size';

export type TerminalKeyAction =
  | 'tab'
  | 'copy'
  | 'paste'
  | 'find'
  | 'find-next'
  | 'find-previous'
  | 'font-up'
  | 'font-down'
  | 'font-reset'
  | 'pass';

export interface TerminalKeyInput {
  type: string;
  key: string;
  code: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

function modifierChord(event: TerminalKeyInput): 'ctrl' | 'meta' | 'none' {
  if (event.altKey) return 'none';
  if (event.ctrlKey && !event.metaKey) return 'ctrl';
  if (event.metaKey && !event.ctrlKey) return 'meta';
  return 'none';
}

export function clampTerminalFontSize(value: number): number {
  if (!Number.isFinite(value)) return TERMINAL_FONT_SIZE_DEFAULT;
  return Math.min(TERMINAL_FONT_SIZE_MAX, Math.max(TERMINAL_FONT_SIZE_MIN, Math.round(value)));
}

export function readStoredTerminalFontSize(): number {
  try {
    const raw = globalThis.localStorage?.getItem(TERMINAL_FONT_SIZE_STORAGE_KEY);
    if (raw === null || raw === undefined) return TERMINAL_FONT_SIZE_DEFAULT;
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? clampTerminalFontSize(parsed) : TERMINAL_FONT_SIZE_DEFAULT;
  } catch {
    return TERMINAL_FONT_SIZE_DEFAULT;
  }
}

export function saveTerminalFontSize(value: number): number {
  const next = clampTerminalFontSize(value);
  try {
    globalThis.localStorage?.setItem(TERMINAL_FONT_SIZE_STORAGE_KEY, String(next));
  } catch {
    // A blocked storage API should not prevent changing the in-memory font.
  }
  return next;
}

export function terminalKeyAction(event: TerminalKeyInput): TerminalKeyAction {
  if (event.type !== 'keydown') return 'pass';
  if (event.key === 'Tab' && !event.ctrlKey && !event.metaKey && !event.altKey) return 'tab';

  const chord = modifierChord(event);
  if (chord === 'none') {
    if (event.key === 'F3') return event.shiftKey ? 'find-previous' : 'find-next';
    return 'pass';
  }

  const letter = event.key.length === 1 ? event.key.toLowerCase() : event.key;
  if (letter === 'c' || event.code === 'KeyC') {
    if (chord === 'meta' && !event.shiftKey) return 'copy';
    if (event.shiftKey) return 'copy';
    return 'pass';
  }
  if (letter === 'v' || event.code === 'KeyV') {
    if (chord === 'meta') return 'paste';
    if (event.shiftKey) return 'paste';
    return 'pass';
  }
  if ((letter === 'f' || event.code === 'KeyF') && !event.shiftKey) return 'find';
  if (
    event.code === 'Equal' ||
    event.code === 'NumpadAdd' ||
    letter === '+' ||
    letter === '='
  ) {
    return 'font-up';
  }
  if (
    (event.code === 'Minus' || event.code === 'NumpadSubtract' || letter === '-') &&
    !event.shiftKey
  ) {
    return 'font-down';
  }
  if (event.code === 'Digit0' || event.code === 'Numpad0' || letter === '0') return 'font-reset';
  return 'pass';
}

export function isSafeHttpUrl(uri: string): boolean {
  if (uri.length === 0 || uri.length > 2_048) return false;
  try {
    const url = new URL(uri);
    return (url.protocol === 'http:' || url.protocol === 'https:') && url.hostname.length > 0;
  } catch {
    return false;
  }
}

export function openSafeHttpUrl(uri: string): void {
  if (!isSafeHttpUrl(uri)) return;
  globalThis.open(uri, '_blank', 'noopener,noreferrer');
}

export function encodeBinaryPtyPayload(data: string): Uint8Array {
  const bytes = new Uint8Array(data.length);
  for (let index = 0; index < data.length; index += 1) {
    bytes[index] = data.charCodeAt(index) & 0xff;
  }
  return bytes;
}

export function safeTerminalTitle(value: string): string | null {
  if (value.length === 0 || value.length > 200) return null;
  if (Array.from(value).some((character) => /\p{Cc}/u.test(character))) return null;
  return value;
}

export function clipboardWriteIsAvailable(): boolean {
  return Boolean(globalThis.isSecureContext && globalThis.navigator?.clipboard?.writeText);
}

export function clipboardReadIsAvailable(): boolean {
  return Boolean(globalThis.isSecureContext && globalThis.navigator?.clipboard?.readText);
}

export async function writeClipboardText(text: string): Promise<boolean> {
  if (text.length === 0) return false;
  if (clipboardWriteIsAvailable()) {
    try {
      await globalThis.navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Fall through to the older copy path used on some private HTTP origins.
    }
  }
  const documentRef = globalThis.document;
  if (documentRef === undefined) return false;
  try {
    const area = documentRef.createElement('textarea');
    area.value = text;
    area.setAttribute('readonly', '');
    area.style.position = 'fixed';
    area.style.left = '-9999px';
    documentRef.body.append(area);
    area.select();
    const copied = documentRef.execCommand('copy');
    area.remove();
    return copied;
  } catch {
    return false;
  }
}

export async function readClipboardText(): Promise<string | null> {
  if (!clipboardReadIsAvailable()) return null;
  try {
    return await globalThis.navigator.clipboard.readText();
  } catch {
    return null;
  }
}
