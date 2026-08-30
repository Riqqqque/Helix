import {
  expectBoolean,
  expectNumber,
  expectRecord,
  requestJson,
} from './api';

export interface SessionExpiryState {
  expires: boolean;
  expiresAtUnixMs: number;
}

function parseSessionExpiry(value: unknown): SessionExpiryState {
  const context = 'Session expiry API';
  const record = expectRecord(value, context);
  return {
    expires: expectBoolean(record, 'expires', context),
    expiresAtUnixMs: expectNumber(record, 'expiresAtUnixMs', context, {
      integer: true,
      minimum: 0,
    }),
  };
}

export function getSessionExpiry(
  csrfToken: string,
  signal?: AbortSignal,
): Promise<SessionExpiryState> {
  return requestJson('/api/v1/auth/session-expiry', parseSessionExpiry, {
    csrfToken,
    signal,
  });
}

export function setSessionExpiry(
  expires: boolean,
  csrfToken: string,
): Promise<SessionExpiryState> {
  return requestJson('/api/v1/auth/session-expiry', parseSessionExpiry, {
    method: 'PUT',
    csrfToken,
    body: { expires },
  });
}
