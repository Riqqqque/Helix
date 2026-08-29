const STORAGE_KEY = 'helix.dismissed.v1';

function readMap(): Record<string, number> {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
    if (raw === null || raw === undefined || raw.length === 0) return {};
    const parsed: unknown = JSON.parse(raw);
    if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) return {};
    const out: Record<string, number> = {};
    for (const [key, value] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof value === 'number' && Number.isFinite(value)) out[key] = value;
    }
    return out;
  } catch {
    return {};
  }
}

export const DISMISSALS_CHANGED_EVENT = 'helix:dismissals-changed';

function emitDismissalsChanged(): void {
  try {
    globalThis.dispatchEvent(new Event(DISMISSALS_CHANGED_EVENT));
  } catch {
    // EventTarget is optional in non-browser tests.
  }
}

function writeMap(map: Record<string, number>): void {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify(map));
  } catch {
    // Preference persistence is optional.
  }
  emitDismissalsChanged();
}

export function isDismissed(id: string): boolean {
  return Object.hasOwn(readMap(), id);
}

export function dismissNotice(id: string): void {
  const map = readMap();
  map[id] = Date.now();
  writeMap(map);
}

export function restoreNotice(id: string): void {
  const map = readMap();
  delete map[id];
  writeMap(map);
}

export function listDismissedIds(): string[] {
  return Object.keys(readMap());
}

export function dismissedCount(): number {
  return listDismissedIds().length;
}

export function clearDismissals(): void {
  writeMap({});
}
