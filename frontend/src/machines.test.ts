import { describe, expect, it } from 'vitest';
import type { Machine, MachineProbe } from './machines-api';
import { machineAddress, machineHealth, machineStatusLabel, machineStatusTone, draftToUpsert } from './machines';

function probeFixture(overrides: Partial<MachineProbe> = {}): MachineProbe {
  return {
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
    swapTotalBytes: null,
    swapFreeBytes: null,
    diskTotalBytes: 1_000_000_000_000,
    diskAvailableBytes: 600_000_000_000,
    processCount: 300,
    containersRunning: 4,
    failedUnits: 0,
    topProcesses: [],
    ...overrides,
  };
}

function machine(probe: Machine['probe']): Machine {
  return {
    id: 'm-1',
    label: 'builder',
    host: '192.0.2.10',
    port: 22,
    username: 'operator',
    authKind: 'key',
    notes: '',
    wolMac: null,
    probe,
    probedAtUnixMs: null,
    createdAtUnixMs: 0,
    updatedAtUnixMs: 0,
  };
}

describe('machine status mapping', () => {
  it('maps probe statuses onto card tones and labels', () => {
    expect(machineStatusTone(machine(null))).toBe('neutral');
    expect(machineStatusLabel(machine(null))).toBe('Not probed');
    for (const [status, tone, label] of [
      ['online', 'good', 'Online'],
      ['reachable', 'warn', 'Port open'],
      ['auth_failed', 'warn', 'Auth failed'],
      ['host_key', 'warn', 'Host key mismatch'],
      ['unreachable', 'bad', 'Unreachable'],
      ['error', 'bad', 'Probe failed'],
    ] as const) {
      expect(machineStatusTone(machine(probeFixture({ status })))).toBe(tone);
      expect(machineStatusLabel(machine(probeFixture({ status })))).toBe(label);
    }
  });

  it('renders the SSH address', () => {
    expect(machineAddress(machine(null))).toBe('operator@192.0.2.10:22');
  });
});

describe('machine health verdict', () => {
  it('reports healthy under normal load', () => {
    const health = machineHealth(probeFixture());
    expect(health?.label).toBe('Healthy');
    expect(health?.tone).toBe('good');
  });

  it('flags busy and overloaded machines', () => {
    const busy = machineHealth(probeFixture({ load: [12, 8, 4], cpuCount: 16 }));
    expect(busy?.label).toBe('Busy');
    const hot = machineHealth(probeFixture({
      memTotalBytes: 100,
      memAvailableBytes: 5,
    }));
    expect(hot?.label).toBe('Overloaded');
    expect(hot?.tone).toBe('bad');
  });

  it('stays silent for offline or unprobed machines', () => {
    expect(machineHealth(null)).toBeNull();
    expect(machineHealth(probeFixture({ status: 'unreachable' }))).toBeNull();
  });
});

describe('machine form draft validation', () => {
  const base = {
    label: 'builder',
    host: '192.0.2.10',
    port: '22',
    username: 'operator',
    authKind: 'key' as const,
    wolMac: '',
    notes: '',
  };

  it('converts a valid draft', () => {
    const upsert = draftToUpsert({ ...base, wolMac: '3C-52-82-AB-12-34' });
    expect(upsert).toMatchObject({ label: 'builder', port: 22, wolMac: '3C-52-82-AB-12-34' });
  });

  it('rejects invalid fields with a readable reason', () => {
    expect(typeof draftToUpsert({ ...base, label: '' })).toBe('string');
    expect(typeof draftToUpsert({ ...base, host: '' })).toBe('string');
    expect(typeof draftToUpsert({ ...base, port: 'abc' })).toBe('string');
    expect(typeof draftToUpsert({ ...base, port: '70000' })).toBe('string');
    expect(typeof draftToUpsert({ ...base, username: '' })).toBe('string');
    expect(typeof draftToUpsert({ ...base, wolMac: 'not-a-mac' })).toBe('string');
  });
});
