import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  clearDismissals,
  DISMISSALS_CHANGED_EVENT,
  dismissNotice,
  dismissedCount,
  isDismissed,
  listDismissedIds,
} from './dismissals';

describe('dismissed notices', () => {
  const values = new Map<string, string>();

  beforeEach(() => {
    values.clear();
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => {
        values.set(key, value);
      },
      removeItem: (key: string) => {
        values.delete(key);
      },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('stores dismissals only in this browser and can show them again', () => {
    const events: string[] = [];
    vi.stubGlobal('dispatchEvent', (event: Event) => {
      events.push(event.type);
      return true;
    });
    dismissNotice('capacity:/');
    dismissNotice('storage-space-intro');
    expect(isDismissed('capacity:/')).toBe(true);
    expect(listDismissedIds()).toEqual(['capacity:/', 'storage-space-intro']);
    expect(dismissedCount()).toBe(2);
    clearDismissals();
    expect(isDismissed('capacity:/')).toBe(false);
    expect(listDismissedIds()).toEqual([]);
    expect(events).toEqual([
      DISMISSALS_CHANGED_EVENT,
      DISMISSALS_CHANGED_EVENT,
      DISMISSALS_CHANGED_EVENT,
    ]);
  });
});
