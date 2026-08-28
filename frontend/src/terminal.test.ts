import { describe, expect, it } from 'vitest';
import { nextTerminalOutputBacklog } from './terminal';

describe('terminal browser resource limits', () => {
  it('accepts a bounded render backlog and rejects invalid or oversized values', () => {
    expect(nextTerminalOutputBacklog(0, 64 * 1024)).toBe(64 * 1024);
    expect(nextTerminalOutputBacklog(4 * 1024 * 1024 - 1, 1)).toBe(4 * 1024 * 1024);
    expect(nextTerminalOutputBacklog(4 * 1024 * 1024, 1)).toBeNull();
    expect(nextTerminalOutputBacklog(-1, 1)).toBeNull();
    expect(nextTerminalOutputBacklog(Number.MAX_SAFE_INTEGER, 1)).toBeNull();
  });
});
