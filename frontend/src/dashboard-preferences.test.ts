import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  moveNavigationItem,
  normalizeNavigationOrder,
  normalizeRefreshInterval,
  readNavigationOrder,
  readRefreshInterval,
  saveNavigationOrder,
  saveRefreshInterval,
} from './dashboard-preferences';

afterEach(() => vi.unstubAllGlobals());

describe('dashboard preferences', () => {
  it('repairs stale, duplicate, and unknown navigation entries', () => {
    expect(normalizeNavigationOrder(['servers', 'overview', 'servers', 'unknown'])).toEqual([
      'servers',
      'overview',
      'home',
      'storage',
      'network',
      'host',
      'security',
      'terminal',
      'hooks',
      'strands',
    ]);
  });

  it('moves navigation without dropping sections or crossing an edge', () => {
    const order = normalizeNavigationOrder(null);
    expect(moveNavigationItem(order, 'home', -1).slice(0, 2)).toEqual(['home', 'overview']);
    expect(moveNavigationItem(order, 'overview', -1)).toEqual(order);
  });

  it('defaults invalid refresh values to one second', () => {
    expect(normalizeRefreshInterval('5000')).toBe(5_000);
    expect(normalizeRefreshInterval(999)).toBe(1_000);
  });

  it('round trips preferences through browser storage', () => {
    const values = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    });
    vi.stubGlobal('dispatchEvent', () => true);

    saveNavigationOrder(['servers', 'hooks', 'terminal', 'host', 'network', 'storage', 'home', 'overview']);
    saveRefreshInterval(2_000);

    expect(readNavigationOrder()[0]).toBe('servers');
    expect(readRefreshInterval()).toBe(2_000);
  });
});
