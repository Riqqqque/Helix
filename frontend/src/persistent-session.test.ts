import { describe, expect, it } from 'vitest';
import {
  clearPersistentSessionProof,
  readPersistentSessionProof,
  restorePersistentSession,
  savePersistentSessionProof,
  syncPersistentSessionProof,
  type SessionProofStorage,
} from './persistent-session';
import { ApiError } from './api';

const TOKEN = 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA';

function memoryStorage(): SessionProofStorage {
  const values = new Map<string, string>();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
}

describe('persistent session proof', () => {
  it('round-trips only a well-formed session-bound proof', () => {
    const storage = memoryStorage();
    expect(savePersistentSessionProof(TOKEN, storage)).toBe(true);
    expect(readPersistentSessionProof(storage)).toBe(TOKEN);
    clearPersistentSessionProof(storage);
    expect(readPersistentSessionProof(storage)).toBeNull();
  });

  it('rejects malformed tokens before they reach browser storage', () => {
    const storage = memoryStorage();
    expect(savePersistentSessionProof('short', storage)).toBe(false);
    expect(readPersistentSessionProof(storage)).toBeNull();
  });

  it.each([false, true])('keeps the refresh proof when server expiry is %s', (sessionExpires) => {
    const storage = memoryStorage();
    expect(syncPersistentSessionProof(TOKEN, sessionExpires, storage)).toBe(true);
    expect(readPersistentSessionProof(storage)).toBe(TOKEN);
  });

  it('removes malformed or obsolete stored values', () => {
    let removed = false;
    const storage: SessionProofStorage = {
      getItem: () => 'obsolete-value',
      setItem: () => undefined,
      removeItem: () => { removed = true; },
    };
    expect(readPersistentSessionProof(storage)).toBeNull();
    expect(removed).toBe(true);
  });

  it('fails closed when browser storage is unavailable', () => {
    const storage: SessionProofStorage = {
      getItem: () => { throw new Error('blocked'); },
      setItem: () => { throw new Error('blocked'); },
      removeItem: () => { throw new Error('blocked'); },
    };
    expect(savePersistentSessionProof(TOKEN, storage)).toBe(false);
    expect(readPersistentSessionProof(storage)).toBeNull();
    expect(() => clearPersistentSessionProof(storage)).not.toThrow();
  });

  it.each([false, true])('restores a current server-confirmed session when expiry is %s', async (sessionExpires) => {
    const storage = memoryStorage();
    savePersistentSessionProof(TOKEN, storage);
    const user = {
      id: '019c7714-3b77-44d1-9866-e1f484aae2ab',
      loginName: 'rique.owner',
      displayName: 'Rique',
      capabilities: ['system.view'],
    };
    await expect(restorePersistentSession(
      { ownerExists: true, bootstrapAvailable: false, bootstrapExpiresAtUnixMs: null },
      undefined,
      () => Promise.resolve({ user, expiresAtUnixMs: 1_900_000_000_000, sessionExpires }),
      storage,
    )).resolves.toEqual({
      user,
      csrfToken: TOKEN,
      expiresAtUnixMs: 1_900_000_000_000,
      sessionExpires,
    });
  });

  it('discards proofs rejected by the server but preserves them on network failure', async () => {
    const status = { ownerExists: true, bootstrapAvailable: false, bootstrapExpiresAtUnixMs: null };
    const storage = memoryStorage();
    savePersistentSessionProof(TOKEN, storage);
    await expect(restorePersistentSession(
      status,
      undefined,
      () => Promise.reject(new ApiError('Rejected.', 403, 'csrf_rejected')),
      storage,
    )).resolves.toBeNull();
    expect(readPersistentSessionProof(storage)).toBeNull();

    savePersistentSessionProof(TOKEN, storage);
    await expect(restorePersistentSession(
      status,
      undefined,
      () => Promise.reject(new ApiError('Offline.')),
      storage,
    )).rejects.toThrow('Offline.');
    expect(readPersistentSessionProof(storage)).toBe(TOKEN);
  });
});
