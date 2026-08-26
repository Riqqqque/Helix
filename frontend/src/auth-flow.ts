import type { TranslationId } from './i18n';
import type { AuthSession, SetupStatus } from './types';

export const loginNoticeIds = {
  sessionExpired: 'auth.login.notice.sessionExpired',
  setupCompleted: 'auth.login.notice.setupCompleted',
} as const satisfies Record<string, TranslationId>;

export type LoginNoticeId = (typeof loginNoticeIds)[keyof typeof loginNoticeIds];

export type AppView =
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  | { kind: 'setup'; status: SetupStatus }
  | { kind: 'login'; notice: LoginNoticeId | null }
  | { kind: 'dashboard'; session: AuthSession };

export type SetupStatusDestination =
  | { kind: 'setup'; status: SetupStatus }
  | { kind: 'login'; notice: null };

export interface SetupCandidate {
  bootstrapToken: string;
  loginName: string;
  displayName: string;
  password: string;
  confirmation: string;
}

export type SetupValidationErrorId = Extract<
  TranslationId,
  `auth.setup.error.${string}`
>;

const MIN_PASSWORD_CODE_POINTS = 15;
const MAX_PASSWORD_CODE_POINTS = 256;
const MAX_PASSWORD_BYTES = 1_024;
const MAX_DISPLAY_NAME_CODE_POINTS = 128;
const MAX_DISPLAY_NAME_BYTES = 512;

function codePointLength(value: string): number {
  return Array.from(value).length;
}

function utf8Length(value: string): number {
  return new TextEncoder().encode(value).length;
}

export function destinationForSetupStatus(status: SetupStatus): SetupStatusDestination {
  return status.ownerExists
    ? { kind: 'login', notice: null }
    : { kind: 'setup', status };
}

export function viewAfterSetupConflict(status: SetupStatus): AppView {
  if (status.ownerExists) {
    return { kind: 'login', notice: loginNoticeIds.setupCompleted };
  }
  return { kind: 'setup', status };
}

export function sessionExpiredView(): AppView {
  return { kind: 'login', notice: loginNoticeIds.sessionExpired };
}

export function validateSetupCandidate(
  candidate: SetupCandidate,
): SetupValidationErrorId | null {
  if (!/^[A-Za-z0-9_-]{43}$/.test(candidate.bootstrapToken)) {
    return 'auth.setup.error.bootstrapToken';
  }
  if (!/^[a-z0-9](?:[a-z0-9._-]{1,62}[a-z0-9])$/.test(candidate.loginName)) {
    return 'auth.setup.error.loginName';
  }
  if (candidate.password !== candidate.confirmation) {
    return 'auth.setup.error.passwordMismatch';
  }

  if (utf8Length(candidate.password) > MAX_PASSWORD_BYTES) {
    return 'auth.setup.error.passwordTooLarge';
  }
  const normalizedPassword = candidate.password.normalize('NFC');
  const passwordCodePoints = codePointLength(normalizedPassword);
  if (passwordCodePoints < MIN_PASSWORD_CODE_POINTS) {
    return 'auth.setup.error.passwordTooShort';
  }
  if (passwordCodePoints > MAX_PASSWORD_CODE_POINTS) {
    return 'auth.setup.error.passwordTooLong';
  }
  if (utf8Length(normalizedPassword) > MAX_PASSWORD_BYTES) {
    return 'auth.setup.error.passwordTooLarge';
  }

  if (utf8Length(candidate.displayName) > MAX_DISPLAY_NAME_BYTES) {
    return 'auth.setup.error.displayNameTooLarge';
  }
  const normalizedDisplayName = candidate.displayName.normalize('NFC');
  const displayNameCodePoints = codePointLength(normalizedDisplayName);
  if (displayNameCodePoints === 0 || displayNameCodePoints > MAX_DISPLAY_NAME_CODE_POINTS) {
    return 'auth.setup.error.displayNameLength';
  }
  if (utf8Length(normalizedDisplayName) > MAX_DISPLAY_NAME_BYTES) {
    return 'auth.setup.error.displayNameTooLarge';
  }
  if (normalizedDisplayName.trim() !== normalizedDisplayName) {
    return 'auth.setup.error.displayNameWhitespace';
  }
  if (Array.from(normalizedDisplayName).some((character) => /\p{Cc}/u.test(character))) {
    return 'auth.setup.error.displayNameControl';
  }

  return null;
}
