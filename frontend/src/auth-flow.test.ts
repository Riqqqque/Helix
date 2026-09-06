import { describe, expect, it } from 'vitest';
import {
  accountUpdatedView,
  destinationForSetupStatus,
  loginNoticeIds,
  sessionExpiredView,
  validateSetupCandidate,
  viewAfterSetupConflict,
} from './auth-flow';
import type { SetupStatus } from './types';

const ownerMissing: SetupStatus = {
  ownerExists: false,
  bootstrapAvailable: true,
  bootstrapExpiresAtUnixMs: 1_800_000_000_000,
};
const ownerPresent: SetupStatus = {
  ownerExists: true,
  bootstrapAvailable: false,
  bootstrapExpiresAtUnixMs: null,
};

describe('authentication flow transitions', () => {
  it('routes a fresh installation to setup and requires login after a reload', () => {
    expect(destinationForSetupStatus(ownerMissing)).toEqual({
      kind: 'setup',
      status: ownerMissing,
    });
    expect(destinationForSetupStatus(ownerPresent)).toEqual({
      kind: 'login',
      notice: null,
    });
  });

  it('recovers a stale setup conflict without leaving the user trapped', () => {
    expect(viewAfterSetupConflict(ownerPresent)).toEqual({
      kind: 'login',
      notice: loginNoticeIds.setupCompleted,
    });
    expect(viewAfterSetupConflict(ownerMissing)).toEqual({
      kind: 'setup',
      status: ownerMissing,
    });
  });

  it('uses an explicit notice when a dashboard session expires', () => {
    expect(sessionExpiredView()).toEqual({
      kind: 'login',
      notice: loginNoticeIds.sessionExpired,
    });
  });

  it('uses a distinct notice after account changes revoke the session', () => {
    expect(accountUpdatedView()).toEqual({
      kind: 'login',
      notice: loginNoticeIds.accountUpdated,
    });
  });
});

describe('owner setup validation', () => {
  const validCandidate = {
    bootstrapToken: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
    loginName: 'rique.owner',
    displayName: 'Rique',
    password: 'a private passphrase with symbols !',
    confirmation: 'a private passphrase with symbols !',
  };

  it('accepts canonical input and rejects mismatched or short passwords', () => {
    const minimumPassword = ['cobalt', 'sky', '92'].join('-');
    expect(validateSetupCandidate(validCandidate)).toBeNull();
    expect(
      validateSetupCandidate({
        ...validCandidate,
        password: minimumPassword,
        confirmation: minimumPassword,
      }),
    ).toBeNull();
    expect(
      validateSetupCandidate({ ...validCandidate, confirmation: 'something else entirely' }),
    ).toBe('auth.setup.error.passwordMismatch');
    expect(
      validateSetupCandidate({
        ...validCandidate,
        password: 'cobalt-sky-9',
        confirmation: 'cobalt-sky-9',
      }),
    ).toBe('auth.setup.error.passwordTooShort');
  });

  it('rejects malformed setup tokens and noncanonical login names', () => {
    expect(
      validateSetupCandidate({ ...validCandidate, bootstrapToken: 'short' }),
    ).toBe('auth.setup.error.bootstrapToken');
    expect(
      validateSetupCandidate({ ...validCandidate, loginName: 'Rique Owner' }),
    ).toBe('auth.setup.error.loginName');
  });

  it('enforces the server code-point bounds before submission', () => {
    const longPassword = 'x'.repeat(257);
    expect(
      validateSetupCandidate({
        ...validCandidate,
        password: longPassword,
        confirmation: longPassword,
      }),
    ).toBe('auth.setup.error.passwordTooLong');
    expect(
      validateSetupCandidate({ ...validCandidate, displayName: 'x'.repeat(129) }),
    ).toBe('auth.setup.error.displayNameLength');
  });

  it('mirrors the server raw UTF-8 safety bounds around normalization', () => {
    const oversizedPassword = '각'.repeat(256);
    expect(
      validateSetupCandidate({
        ...validCandidate,
        password: oversizedPassword,
        confirmation: oversizedPassword,
      }),
    ).toBe('auth.setup.error.passwordTooLarge');
    expect(
      validateSetupCandidate({ ...validCandidate, displayName: '각'.repeat(128) }),
    ).toBe('auth.setup.error.displayNameTooLarge');
  });

  it('admits supplementary Unicode at the canonical byte boundaries', () => {
    expect(
      validateSetupCandidate({ ...validCandidate, displayName: '😀'.repeat(128) }),
    ).toBeNull();
    const boundaryPassword = '😀'.repeat(256);
    expect(
      validateSetupCandidate({
        ...validCandidate,
        password: boundaryPassword,
        confirmation: boundaryPassword,
      }),
    ).toBeNull();
  });

  it('rejects ambiguous display-name boundaries and controls', () => {
    expect(
      validateSetupCandidate({ ...validCandidate, displayName: ' Rique' }),
    ).toBe('auth.setup.error.displayNameWhitespace');
    expect(
      validateSetupCandidate({ ...validCandidate, displayName: 'Rique\tOwner' }),
    ).toBe('auth.setup.error.displayNameControl');
  });
});
