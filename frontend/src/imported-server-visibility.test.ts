import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  forgetImportedServer,
  readForgottenImportedServers,
  readHiddenImportedServers,
  rememberImportedServer,
} from './imported-server-visibility';

describe('imported server visibility', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('forgets a hidden AMP connection in this browser without inventing an AMP delete', () => {
    const store = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => {
        store.set(key, value);
      },
    });

    const id = 'amp:71b629b7-5861-47b8-907b-acde40dadc9e';
    const next = forgetImportedServer(id, [id], []);
    expect(next.hidden).toEqual([]);
    expect(next.forgotten).toEqual([id]);
    expect(readHiddenImportedServers()).toEqual([]);
    expect(readForgottenImportedServers()).toEqual([id]);

    const restored = rememberImportedServer(id, next.hidden, next.forgotten);
    expect(restored.forgotten).toEqual([]);
    expect(readForgottenImportedServers()).toEqual([]);
  });
});
