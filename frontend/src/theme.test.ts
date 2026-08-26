import { describe, expect, it } from 'vitest';
import { normalizeTheme, resolveTheme } from './theme';

describe('theme preferences', () => {
  it('falls back to the system preference for unknown stored values', () => {
    expect(normalizeTheme('sepia')).toBe('system');
    expect(normalizeTheme(null)).toBe('system');
  });

  it('keeps every supported explicit theme', () => {
    expect(normalizeTheme('midnight')).toBe('midnight');
    expect(normalizeTheme('oled')).toBe('oled');
    expect(normalizeTheme('light')).toBe('light');
  });

  it('resolves system mode without changing explicit choices', () => {
    expect(resolveTheme('system', true)).toBe('light');
    expect(resolveTheme('system', false)).toBe('midnight');
    expect(resolveTheme('oled', true)).toBe('oled');
  });
});
