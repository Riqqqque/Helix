import { describe, expect, it } from 'vitest';
import {
  forgetResumableOperation,
  beginMutationRequest,
  endMutationRequest,
  mutationRefreshIsUnsafe,
  readResumableOperations,
  rememberOperationDispatch,
  updateResumableOperation,
} from './operation-continuity';

function storage() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
  };
}

const id = '12345678-1234-4234-8234-123456789abc';

describe('operation continuity journal', () => {
  it('captures only an opaque accepted job receipt', () => {
    const memory = storage();
    rememberOperationDispatch('/api/v1/servers/minecraft/modpacks', { jobId: id, secret: 'not stored' }, memory, 1_000);
    expect(readResumableOperations(memory, 1_000)).toEqual([expect.objectContaining({
      id,
      label: 'Minecraft modpack creation',
      status: 'queued',
    })]);
    expect(memory.getItem('helix.operations.v1')).not.toContain('secret');
  });

  it('updates status without resubmitting the original action', () => {
    const memory = storage();
    const now = Date.now();
    rememberOperationDispatch('/api/v1/servers/minecraft', { job_id: id }, memory, now);
    updateResumableOperation({
      id,
      kind: 'minecraft_create',
      status: 'running',
      stage: 'Starting Minecraft',
      progressPercent: 82,
      createdAtUnixMs: now,
      updatedAtUnixMs: now + 1_000,
      result: null,
      error: null,
    }, memory);
    expect(readResumableOperations(memory, now + 1_000)[0]).toMatchObject({
      status: 'running',
      stage: 'Starting Minecraft',
      progressPercent: 82,
    });
    forgetResumableOperation(id, memory);
    expect(readResumableOperations(memory, now + 1_000)).toEqual([]);
  });

  it('ignores malformed, stale, and separately resumable storage analysis jobs', () => {
    const memory = storage();
    rememberOperationDispatch('/api/v1/storage/analysis/start', { jobId: id }, memory, 1_000);
    rememberOperationDispatch('/api/v1/servers/minecraft', { jobId: 'bad' }, memory, 1_000);
    expect(readResumableOperations(memory, 1_000)).toEqual([]);
  });

  it('tracks overlapping mutation requests until every response is settled', () => {
    expect(mutationRefreshIsUnsafe()).toBe(false);
    beginMutationRequest();
    beginMutationRequest();
    expect(mutationRefreshIsUnsafe()).toBe(true);
    endMutationRequest();
    expect(mutationRefreshIsUnsafe()).toBe(true);
    endMutationRequest();
    expect(mutationRefreshIsUnsafe()).toBe(false);
  });
});
