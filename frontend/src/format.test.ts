import { describe, expect, it } from 'vitest';
import {
  calculatePercent,
  formatBytes,
  formatDuration,
  formatPercent,
} from './format';

describe('formatBytes', () => {
  it('formats binary units without overstating precision', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(1_024)).toBe('1 KiB');
    expect(formatBytes(1_073_741_824)).toBe('1 GiB');
  });

  it('formats full-range bigint counters without converting the source value to number', () => {
    expect(formatBytes(1_024n)).toBe('1 KiB');
    expect(formatBytes(1_536n)).toBe('1.5 KiB');
    expect(formatBytes(18_446_744_073_709_551_615n)).toBe('16 EiB');
  });

  it('does not display invalid values as real measurements', () => {
    expect(formatBytes(-1)).toBe('Unavailable');
    expect(formatBytes(-1n)).toBe('Unavailable');
    expect(formatBytes(Number.NaN)).toBe('Unavailable');
  });
});

describe('formatDuration', () => {
  it('uses compact host uptime units', () => {
    expect(formatDuration(42)).toBe('42s');
    expect(formatDuration(125)).toBe('2m 5s');
    expect(formatDuration(7_380)).toBe('2h 3m');
    expect(formatDuration(90_061)).toBe('1d 1h');
  });
});

describe('percent helpers', () => {
  it('calculates percentages only with a real total', () => {
    expect(calculatePercent(25, 100)).toBe(25);
    expect(calculatePercent(0, 0)).toBeNull();
    expect(calculatePercent(25n, 100n)).toBe(25);
    expect(calculatePercent(1n, 0n)).toBeNull();
  });

  it('formats percentages consistently', () => {
    expect(formatPercent(12.5)).toBe('12.5%');
  });
});
