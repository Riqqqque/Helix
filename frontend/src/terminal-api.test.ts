import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  getTerminalStatus,
  parseTerminalStatus,
  parseTerminalTicket,
  requestTerminalTicket,
  terminalWebSocketUrl,
} from './terminal-api';

afterEach(() => vi.unstubAllGlobals());

describe('terminal API', () => {
  it('accepts only the fixed privilege, persistence, path, and subprotocol contract', () => {
    expect(parseTerminalStatus({
      availability: 'available',
      reauthenticationRequired: true,
      shellPrivilege: 'linux_user',
      persistence: 'while_connected',
      detail: 'Ready.',
    }).availability).toBe('available');
    expect(() => parseTerminalStatus({
      availability: 'available',
      reauthenticationRequired: false,
      shellPrivilege: 'root',
      persistence: 'forever',
      detail: 'Unsafe.',
    })).toThrow();
    expect(parseTerminalTicket({
      expiresAtUnixMs: 1_900_000_000_000,
      connectPath: '/api/v1/terminal/connect',
      subprotocol: 'helix-terminal-v1',
    }).connectPath).toBe('/api/v1/terminal/connect');
    expect(() => parseTerminalTicket({
      expiresAtUnixMs: 1_900_000_000_000,
      connectPath: '/api/v1/terminal/connect?ticket=secret',
      subprotocol: 'helix-terminal-v1',
    })).toThrow();
  });

  it('sends current-password proof in same-origin JSON and never returns it in a URL', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({
      expiresAtUnixMs: 1_900_000_000_000,
      connectPath: '/api/v1/terminal/connect',
      subprotocol: 'helix-terminal-v1',
    }), { status: 201, headers: { 'Content-Type': 'application/json' } }));
    vi.stubGlobal('fetch', fetchMock);
    await requestTerminalTicket('current password only', 120, 36, 'csrf-proof');
    const [path, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(path).toBe('/api/v1/terminal/ticket');
    expect(path).not.toContain('current password only');
    expect(request.credentials).toBe('same-origin');
    expect(request.headers).toMatchObject({ 'X-Helix-CSRF': 'csrf-proof' });
    expect(request.body).toBe(JSON.stringify({
      currentPassword: 'current password only',
      columns: 120,
      rows: 36,
    }));
  });

  it('uses protected status reads and upgrades only the fixed same-origin path', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({
      availability: 'unavailable',
      reauthenticationRequired: true,
      shellPrivilege: 'linux_user',
      persistence: 'while_connected',
      detail: 'Not configured.',
    }), { status: 200, headers: { 'Content-Type': 'application/json' } }));
    vi.stubGlobal('fetch', fetchMock);
    await getTerminalStatus('csrf-proof');
    expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({
      credentials: 'same-origin',
      headers: expect.objectContaining({ 'X-Helix-CSRF': 'csrf-proof' }),
    });
    expect(terminalWebSocketUrl('/api/v1/terminal/connect', 'http://192.168.1.5:3100/#terminal'))
      .toBe('ws://192.168.1.5:3100/api/v1/terminal/connect');
    expect(terminalWebSocketUrl('/api/v1/terminal/connect', 'https://helix.local/#terminal'))
      .toBe('wss://helix.local/api/v1/terminal/connect');
  });
});
