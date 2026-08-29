import { describe, expect, it } from 'vitest';
import { nextTerminalOutputBacklog, parseHostTerminalEvent } from './terminal';

describe('terminal browser resource limits', () => {
  it('accepts a bounded render backlog and rejects invalid or oversized values', () => {
    expect(nextTerminalOutputBacklog(0, 64 * 1024)).toBe(64 * 1024);
    expect(nextTerminalOutputBacklog(4 * 1024 * 1024 - 1, 1)).toBe(4 * 1024 * 1024);
    expect(nextTerminalOutputBacklog(4 * 1024 * 1024, 1)).toBeNull();
    expect(nextTerminalOutputBacklog(-1, 1)).toBeNull();
    expect(nextTerminalOutputBacklog(Number.MAX_SAFE_INTEGER, 1)).toBeNull();
  });
});

describe('host terminal events', () => {
  it('accepts bounded ready, exit, and error payloads', () => {
    expect(parseHostTerminalEvent('{"type":"heartbeat"}')).toEqual({ type: 'heartbeat' });
    expect(parseHostTerminalEvent('{"type":"ready","user":"rique","shell":"/bin/bash"}')).toEqual({
      type: 'ready',
      user: 'rique',
      shell: '/bin/bash',
    });
    expect(parseHostTerminalEvent('{"type":"exit","exitCode":0,"signal":null}')).toEqual({
      type: 'exit',
      exitCode: 0,
      signal: null,
    });
    expect(parseHostTerminalEvent('{"type":"error","message":"The shell ended."}')).toEqual({
      type: 'error',
      message: 'The shell ended.',
    });
  });

  it('rejects extra fields, relative shells, and control characters', () => {
    expect(parseHostTerminalEvent('{"type":"ready","user":"rique","shell":"bash"}')).toBeNull();
    expect(parseHostTerminalEvent('{"type":"ready","user":"rique","shell":"/bin/bash","extra":1}')).toBeNull();
    expect(parseHostTerminalEvent('{"type":"error","message":"bad\\u0007"}')).toBeNull();
  });
});
