import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  TERMINAL_FONT_SIZE_DEFAULT,
  TERMINAL_FONT_SIZE_MAX,
  TERMINAL_FONT_SIZE_MIN,
  TERMINAL_FONT_SIZE_STORAGE_KEY,
  clampTerminalFontSize,
  encodeBinaryPtyPayload,
  isSafeHttpUrl,
  readStoredTerminalFontSize,
  safeTerminalTitle,
  saveTerminalFontSize,
  terminalKeyAction,
  type TerminalKeyInput,
} from './terminal-keys';

afterEach(() => vi.unstubAllGlobals());

function key(partial: Partial<TerminalKeyInput> & Pick<TerminalKeyInput, 'key'>): TerminalKeyInput {
  return {
    type: 'keydown',
    code: '',
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    ...partial,
  };
}

describe('terminal key capture', () => {
  it('keeps Tab and Shift+Tab in the PTY instead of moving browser focus', () => {
    expect(terminalKeyAction(key({ key: 'Tab', code: 'Tab' }))).toBe('tab');
    expect(terminalKeyAction(key({ key: 'Tab', code: 'Tab', shiftKey: true }))).toBe('tab');
    expect(terminalKeyAction(key({ key: 'Tab', code: 'Tab', ctrlKey: true }))).toBe('pass');
  });

  it('copies and pastes with Shift+Ctrl/Cmd without stealing SIGINT', () => {
    expect(terminalKeyAction(key({ key: 'c', code: 'KeyC', ctrlKey: true }))).toBe('pass');
    expect(terminalKeyAction(key({ key: 'c', code: 'KeyC', ctrlKey: true, shiftKey: true }))).toBe('copy');
    expect(terminalKeyAction(key({ key: 'c', code: 'KeyC', metaKey: true }))).toBe('copy');
    expect(terminalKeyAction(key({ key: 'v', code: 'KeyV', ctrlKey: true }))).toBe('pass');
    expect(terminalKeyAction(key({ key: 'v', code: 'KeyV', ctrlKey: true, shiftKey: true }))).toBe('paste');
    expect(terminalKeyAction(key({ key: 'v', code: 'KeyV', metaKey: true }))).toBe('paste');
  });

  it('opens find and changes font size from the usual browser shortcuts', () => {
    expect(terminalKeyAction(key({ key: 'f', code: 'KeyF', ctrlKey: true }))).toBe('find');
    expect(terminalKeyAction(key({ key: 'F3', code: 'F3' }))).toBe('find-next');
    expect(terminalKeyAction(key({ key: 'F3', code: 'F3', shiftKey: true }))).toBe('find-previous');
    expect(terminalKeyAction(key({ key: '=', code: 'Equal', ctrlKey: true }))).toBe('font-up');
    expect(terminalKeyAction(key({ key: '-', code: 'Minus', ctrlKey: true }))).toBe('font-down');
    expect(terminalKeyAction(key({ key: '_', code: 'Minus', ctrlKey: true, shiftKey: true }))).toBe('pass');
    expect(terminalKeyAction(key({ key: '0', code: 'Digit0', ctrlKey: true }))).toBe('font-reset');
    expect(terminalKeyAction(key({ type: 'keyup', key: 'Tab', code: 'Tab' }))).toBe('pass');
  });
});

describe('terminal helpers', () => {
  it('clamps and stores font size', () => {
    expect(clampTerminalFontSize(Number.NaN)).toBe(TERMINAL_FONT_SIZE_DEFAULT);
    expect(clampTerminalFontSize(8)).toBe(TERMINAL_FONT_SIZE_MIN);
    expect(clampTerminalFontSize(48)).toBe(TERMINAL_FONT_SIZE_MAX);
    const values = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      getItem: (storageKey: string) => values.get(storageKey) ?? null,
      setItem: (storageKey: string, value: string) => {
        values.set(storageKey, value);
      },
    });
    expect(saveTerminalFontSize(18)).toBe(18);
    expect(values.get(TERMINAL_FONT_SIZE_STORAGE_KEY)).toBe('18');
    expect(readStoredTerminalFontSize()).toBe(18);
  });

  it('opens only real http(s) links', () => {
    expect(isSafeHttpUrl('https://example.com/docs')).toBe(true);
    expect(isSafeHttpUrl('http://192.168.1.10:8080/')).toBe(true);
    expect(isSafeHttpUrl('javascript:alert(1)')).toBe(false);
    expect(isSafeHttpUrl('file:///etc/passwd')).toBe(false);
    expect(isSafeHttpUrl('ftp://example.com/file')).toBe(false);
    expect(isSafeHttpUrl('https://')).toBe(false);
  });

  it('encodes mouse reports as bytes and rejects unsafe window titles', () => {
    expect(Array.from(encodeBinaryPtyPayload('A\u00ff'))).toEqual([65, 255]);
    expect(safeTerminalTitle('rique@host: ~/src')).toBe('rique@host: ~/src');
    expect(safeTerminalTitle('bad\u0007title')).toBeNull();
    expect(safeTerminalTitle('')).toBeNull();
  });
});
