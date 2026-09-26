import render from 'preact-render-to-string';
import { describe, expect, it, vi } from 'vitest';
import { parseExtraPorts, setNativeExtraPorts } from './control-api';
import { ServerPortsCard, followSavedPorts, portProfile, validatePortDraft } from './server-ports';

const base = {
  serverId: 'helix:test',
  kind: 'minecraft',
  gamePort: 25_566,
  queryPort: null,
  software: 'Paper',
  lanAddress: '192.168.1.20',
  running: true,
  csrfToken: 'csrf',
  canManageServers: true,
  onSaved: () => undefined,
  onSessionExpired: () => undefined,
};

describe('server ports card', () => {
  it('shows the game port, saved extra ports with their LAN address, and voice chat presets', () => {
    const html = render(<ServerPortsCard {...base} extraPorts={[{ port: 8_100, protocol: 'tcp', label: 'BlueMap' }]} />);
    expect(html).toContain('Ports');
    expect(html).toContain('25566');
    expect(html).toContain('192.168.1.20:8100');
    expect(html).toContain('Simple Voice Chat');
    expect(html).toContain('24454');
    expect(html).not.toContain('+ BlueMap');
    expect(html).toContain('restarts the server');
  });

  it('offers presets only for Minecraft and locks editing without permission', () => {
    const html = render(<ServerPortsCard {...base} kind="valheim" software="Valheim" canManageServers={false} extraPorts={[]} />);
    expect(html).not.toContain('Simple Voice Chat');
    expect(html).toContain('No extra ports yet.');
    expect(html).toContain('Requires games.manage permission');
  });

  it('rejects privileged, duplicate, reserved and oversized drafts', () => {
    expect(validatePortDraft([{ port: 24_454, protocol: 'udp', label: 'Voice' }], [25_565])).toBeNull();
    expect(validatePortDraft([{ port: 80, protocol: 'tcp', label: '' }], [])).toContain('1024');
    expect(validatePortDraft([{ port: 25_565, protocol: 'udp', label: '' }], [25_565])).toContain('game or query port');
    expect(validatePortDraft([{ port: 24_454, protocol: 'udp', label: '' }, { port: 24_454, protocol: 'tcp', label: '' }], [])).toContain('listed twice');
    expect(validatePortDraft([{ port: 24_454, protocol: 'udp', label: 'x'.repeat(41) }], [])).toContain('40');
    expect(validatePortDraft(Array.from({ length: 17 }, (_, index) => ({ port: 30_000 + index, protocol: 'tcp' as const, label: '' })), [])).toContain('16');
  });

  it('parses only well-formed ports and sends the ports route', async () => {
    expect(parseExtraPorts([{ port: 24454, protocol: 'udp', label: 'Voice' }, { port: 'x', protocol: 'udp' }, { port: 9000, protocol: 'sctp' }])).toEqual([
      { port: 24_454, protocol: 'udp', label: 'Voice' },
    ]);
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({ changed: true, was_running: true, extra_ports: [{ port: 24454, protocol: 'udp', label: 'Voice' }] }), { status: 200, headers: { 'content-type': 'application/json' } }));
    vi.stubGlobal('fetch', fetchMock);
    await expect(setNativeExtraPorts('helix:test', [{ port: 24_454, protocol: 'udp', label: ' Voice ' }], 'csrf')).resolves.toEqual({
      changed: true,
      wasRunning: true,
      extraPorts: [{ port: 24_454, protocol: 'udp', label: 'Voice' }],
    });
    const [path, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(path).toContain('helix%3Atest/ports');
    expect(request.method).toBe('PUT');
    expect(JSON.parse(String(request.body))).toEqual({ ports: [{ port: 24_454, protocol: 'udp', label: 'Voice' }] });
    vi.unstubAllGlobals();
  });
  it('matches presets and protocols to each game and server software', () => {
    const labels = (kind: string, software: string) => portProfile(kind, software).presets.map((preset) => preset.label);
    expect(labels('minecraft', 'Paper')).toEqual(['Simple Voice Chat', 'BlueMap', 'Dynmap', 'Geyser (Bedrock)']);
    expect(portProfile('minecraft', 'Paper').configNote).toBe('plugins/');
    expect(portProfile('minecraft', 'Fabric').configNote).toBe('config/');
    expect(labels('minecraft', 'Vanilla')).toEqual([]);
    expect(labels('minecraft', 'Pumpkin')).toEqual([]);
    expect(portProfile('minecraft', 'Pumpkin').queryLabel).toContain('Bedrock');
    expect(labels('hytale', 'Hytale')).toEqual([]);
    expect(portProfile('hytale', 'Hytale').gameProtocol).toBe('UDP (QUIC)');
    expect(portProfile('satisfactory', 'Satisfactory')).toMatchObject({ gameProtocol: 'UDP', queryLabel: 'Beacon UDP' });
    expect(portProfile('vintage_story', 'Vintage Story').gameProtocol).toBe('TCP');
    const hytale = render(<ServerPortsCard {...base} kind="hytale" software="Hytale" gamePort={5_520} extraPorts={[]} />);
    expect(hytale).not.toContain('Simple Voice Chat');
    expect(hytale).not.toContain('BlueMap');
    expect(hytale).toContain('UDP (QUIC)');
  });
});

describe('ports draft across refreshes', () => {
  const voice = { port: 24_454, protocol: 'udp' as const, label: 'Simple Voice Chat' };
  const map = { port: 8_100, protocol: 'tcp' as const, label: 'BlueMap' };
  it('keeps a clicked preset when the page refreshes with the same saved ports', () => {
    const saved: typeof voice[] = [];
    const draft = [voice];
    expect(followSavedPorts(draft, saved, [])).toEqual([voice]);
  });
  it('follows a real saved change when nothing is being edited', () => {
    expect(followSavedPorts([], [], [map])).toEqual([map]);
    expect(followSavedPorts([map], [map], [])).toEqual([]);
  });
  it('does not overwrite unsaved edits when the saved ports change elsewhere', () => {
    expect(followSavedPorts([voice], [], [map])).toEqual([voice]);
  });
});
