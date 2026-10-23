import { describe, expect, it } from 'vitest';
import { formatMemoryInput, parseMemoryInput } from './memory-input';

describe('memory input', () => {
  it('reads sizes the way people type them', () => {
    expect(parseMemoryInput('6144')).toBe(6_144);
    expect(parseMemoryInput('6')).toBe(6_144);
    expect(parseMemoryInput('6G')).toBe(6_144);
    expect(parseMemoryInput('6 gb')).toBe(6_144);
    expect(parseMemoryInput('6.5GiB')).toBe(6_656);
    expect(parseMemoryInput('6,5 GB')).toBe(6_656);
    expect(parseMemoryInput('512m')).toBe(512);
    expect(parseMemoryInput('512 MB')).toBe(512);
    expect(parseMemoryInput('1 TB')).toBe(1_048_576);
    expect(parseMemoryInput('')).toBeNull();
    expect(parseMemoryInput('six')).toBeNull();
    expect(parseMemoryInput('-2G')).toBeNull();
    expect(parseMemoryInput('0')).toBeNull();
  });

  it('formats whole GiB compactly and other sizes in MiB', () => {
    expect(formatMemoryInput(8_192)).toBe('8 GiB');
    expect(formatMemoryInput(6_656)).toBe('6656 MiB');
  });
});
