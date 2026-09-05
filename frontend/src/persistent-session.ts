import { ApiError, getCurrentUser, getSetupStatus } from './api';
import type { AppView } from './auth-flow';
import type { AuthSession, AuthenticatedUserResponse, SetupStatus } from './types';

const STORAGE_KEY = 'helix.auth.persisted-csrf.v1';
const TOKEN_PATTERN = /^[A-Za-z0-9_-]{43}$/u;

export interface SessionProofStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

function browserStorage(): SessionProofStorage | null {
  if (typeof window === 'undefined') return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function readPersistentSessionProof(
  storage: SessionProofStorage | null = browserStorage(),
): string | null {
  if (storage === null) return null;
  try {
    const value = storage.getItem(STORAGE_KEY);
    if (value !== null && !TOKEN_PATTERN.test(value)) {
      storage.removeItem(STORAGE_KEY);
      return null;
    }
    return value;
  } catch {
    try {
      storage.removeItem(STORAGE_KEY);
    } catch {
      // A browser that blocks storage simply falls back to normal sign-in.
    }
    return null;
  }
}

export function savePersistentSessionProof(
  csrfToken: string,
  storage: SessionProofStorage | null = browserStorage(),
): boolean {
  if (storage === null || !TOKEN_PATTERN.test(csrfToken)) return false;
  try {
    storage.setItem(STORAGE_KEY, csrfToken);
    return true;
  } catch {
    return false;
  }
}

export function clearPersistentSessionProof(
  storage: SessionProofStorage | null = browserStorage(),
): void {
  if (storage === null) return;
  try {
    storage.removeItem(STORAGE_KEY);
  } catch {
    // Failure to access storage also prevents Helix from restoring from it.
  }
}

export async function restorePersistentSession(
  status: SetupStatus,
  signal?: AbortSignal,
  currentUser: (csrfToken: string) => Promise<AuthenticatedUserResponse> =
    (csrfToken) => getCurrentUser(csrfToken, signal),
  storage: SessionProofStorage | null = browserStorage(),
): Promise<AuthSession | null> {
  if (!status.ownerExists) {
    clearPersistentSessionProof(storage);
    return null;
  }
  const csrfToken = readPersistentSessionProof(storage);
  if (csrfToken === null) return null;
  try {
    const response = await currentUser(csrfToken);
    return { ...response, csrfToken };
  } catch (error) {
    if (error instanceof ApiError && (error.status === 401 || error.code === 'csrf_rejected')) {
      clearPersistentSessionProof(storage);
      return null;
    }
    throw error;
  }
}

export function syncPersistentSessionProof(
  csrfToken: string,
  sessionExpires: boolean,
  storage: SessionProofStorage | null = browserStorage(),
): boolean {
  void sessionExpires;
  return savePersistentSessionProof(csrfToken, storage);
}

export async function initialAuthView(signal: AbortSignal): Promise<AppView> {
  const status = await getSetupStatus(signal);
  const session = await restorePersistentSession(status, signal);
  if (session !== null) return { kind: 'dashboard', session };
  return status.ownerExists
    ? { kind: 'login', notice: null }
    : { kind: 'setup', status };
}
