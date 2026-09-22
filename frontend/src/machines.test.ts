import { describe, expect, it } from 'vitest';
import type { Machine } from './machines-api';
import { machineAddress, machineStatusLabel, machineStatusTone, draftToUpsert } from './machines';

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
      const probe = { status, detail: '', latencyMs: null, hostname: null, os: null, kernel: null, uptimeSeconds: null, load: null, cpuCount: null, memTotalBytes: null, memAvailableBytes: null, diskTotalBytes: null, diskAvailableBytes: null, containersRunning: null, failedUnits: null };
      expect(machineStatusTone(machine(probe))).toBe(tone);
      expect(machineStatusLabel(machine(probe))).toBe(label);
    }
  });

  it('renders the SSH address', () => {
    expect(machineAddress(machine(null))).toBe('operator@192.0.2.10:22');
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
