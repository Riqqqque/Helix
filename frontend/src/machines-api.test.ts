import { describe, expect, it } from 'vitest';
import {
  parseHubIdentity,
  parseMachine,
  parseMachineProbe,
} from './machines-api';

const probeFixture = {
  status: 'online',
  detail: 'SSH probe succeeded',
  latencyMs: 42,
  hostname: 'builder',
  os: 'Ubuntu 24.04.3 LTS',
  kernel: '6.8.0-79-generic',
  uptimeSeconds: 1_234_567,
  load: [0.42, 0.5, 0.61],
  cpuCount: 16,
  memTotalBytes: 33_554_432_000,
  memAvailableBytes: 16_777_216_000,
  swapTotalBytes: 8_589_934_592,
  swapFreeBytes: 7_500_000_000,
  diskTotalBytes: 1_000_000_000_000,
  diskAvailableBytes: 600_000_000_000,
  processCount: 412,
  containersRunning: 4,
  failedUnits: 0,
  topProcesses: [
    { name: 'nginx', cpuPercent: 45.2 },
    { name: 'postgres', cpuPercent: 12.1 },
  ],
};

const machineFixture = {
  id: '7a2b0c9d-0000-4000-8000-abcdef012345',
  label: 'builder',
  host: '192.0.2.10',
  port: 22,
  username: 'operator',
  authKind: 'key',
  notes: '',
  wolMac: '3c:52:82:ab:12:34',
  probe: probeFixture,
  probedAtUnixMs: 1_758_000_000_000,
  createdAtUnixMs: 1_758_000_000_000,
  updatedAtUnixMs: 1_758_000_000_000,
};

describe('machine api parsing', () => {
  it('parses a full machine record with a cached probe', () => {
    const machine = parseMachine(machineFixture);
    expect(machine.id).toBe(machineFixture.id);
    expect(machine.authKind).toBe('key');
    expect(machine.probe?.status).toBe('online');
    expect(machine.probe?.load).toEqual([0.42, 0.5, 0.61]);
    expect(machine.probe?.containersRunning).toBe(4);
    expect(machine.probe?.swapTotalBytes).toBe(8_589_934_592);
    expect(machine.probe?.processCount).toBe(412);
    expect(machine.probe?.topProcesses).toEqual([
      { name: 'nginx', cpuPercent: 45.2 },
      { name: 'postgres', cpuPercent: 12.1 },
    ]);
  });

  it('accepts machines without probe data or a WoL address', () => {
    const machine = parseMachine({
      ...machineFixture,
      wolMac: null,
      probe: null,
      probedAtUnixMs: null,
    });
    expect(machine.probe).toBeNull();
    expect(machine.wolMac).toBeNull();
  });

  it('parses password-auth machines', () => {
    expect(parseMachine({ ...machineFixture, authKind: 'password' }).authKind).toBe('password');
    expect(parseMachine({ ...machineFixture, authKind: 'system' }).authKind).toBe('system');
  });

  it('rejects malformed records', () => {
    expect(() => parseMachine({ ...machineFixture, port: 70000 })).toThrow();
    expect(() => parseMachine({ ...machineFixture, authKind: 'agent' })).toThrow();
    expect(() => parseMachine({ ...machineFixture, host: '' })).toThrow();
    expect(() => parseMachine({ ...machineFixture, id: 'x'.repeat(65) })).toThrow();
  });

  it('rejects invalid probe statuses and loads', () => {
    expect(() => parseMachineProbe({ ...probeFixture, status: 'weird' })).toThrow();
    expect(() => parseMachineProbe({ ...probeFixture, load: [0.1] })).toThrow();
    expect(() => parseMachineProbe({ ...probeFixture, load: ['a', 1, 2] })).toThrow();
    expect(() => parseMachineProbe({ ...probeFixture, uptimeSeconds: -1 })).toThrow();
  });

  it('rejects malformed top-process entries', () => {
    expect(() => parseMachineProbe({ ...probeFixture, topProcesses: [{ name: 'x' }] })).toThrow();
    expect(() => parseMachineProbe({ ...probeFixture, topProcesses: [{ name: 'x', cpuPercent: -1 }] })).toThrow();
    expect(() => parseMachineProbe({ ...probeFixture, topProcesses: 'nginx' })).toThrow();
    expect(parseMachineProbe({ ...probeFixture, topProcesses: null }).topProcesses).toEqual([]);
  });

  it('parses the hub identity', () => {
    const identity = parseHubIdentity({
      available: true,
      publicKey: 'ssh-ed25519 AAAAC3NzaC helix-hub',
      fingerprintSha256: 'SHA256:abc123',
      user: 'operator',
      detail: 'ready',
    });
    expect(identity.available).toBe(true);
    expect(identity.fingerprintSha256).toBe('SHA256:abc123');
    expect(parseHubIdentity({ available: false, publicKey: null, fingerprintSha256: null, user: null, detail: 'down' }).available).toBe(false);
  });
});
