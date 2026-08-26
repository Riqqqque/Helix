import type { TranslationId } from './i18n';

export const themeOptions = [
  { value: 'system', labelId: 'theme.system' },
  { value: 'midnight', labelId: 'theme.midnight' },
  { value: 'oled', labelId: 'theme.oled' },
  { value: 'light', labelId: 'theme.light' },
] as const satisfies ReadonlyArray<{ value: string; labelId: TranslationId }>;

export type ThemePreference = (typeof themeOptions)[number]['value'];
export type ResolvedTheme = Exclude<ThemePreference, 'system'>;

const STORAGE_KEY = 'helix.theme';

export function normalizeTheme(value: unknown): ThemePreference {
  return themeOptions.some((option) => option.value === value)
    ? (value as ThemePreference)
    : 'system';
}

export function resolveTheme(
  preference: ThemePreference,
  systemPrefersLight: boolean,
): ResolvedTheme {
  if (preference === 'system') {
    return systemPrefersLight ? 'light' : 'midnight';
  }

  return preference;
}

export function readThemePreference(): ThemePreference {
  try {
    return normalizeTheme(globalThis.localStorage?.getItem(STORAGE_KEY));
  } catch {
    return 'system';
  }
}

export function saveThemePreference(preference: ThemePreference): void {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, preference);
  } catch {
    // A blocked storage API should not prevent changing the in-memory theme.
  }
}

export function applyThemePreference(
  preference: ThemePreference,
  systemPrefersLight = globalThis.matchMedia?.('(prefers-color-scheme: light)')
    .matches ?? false,
): ResolvedTheme {
  const resolved = resolveTheme(preference, systemPrefersLight);
  document.documentElement.dataset.theme = resolved;
  document.documentElement.dataset.themePreference = preference;

  const themeColor =
    resolved === 'light' ? '#f4f7fb' : resolved === 'oled' ? '#000000' : '#090d16';
  document
    .querySelector<HTMLMetaElement>('meta[name="theme-color"]')
    ?.setAttribute('content', themeColor);

  return resolved;
}

export function initializeTheme(): ThemePreference {
  const preference = readThemePreference();
  applyThemePreference(preference);
  return preference;
}
