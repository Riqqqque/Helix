import type { HomeWidget, HomeWidgetHeight, HomeWidgetKind, HomeWidgetSize } from './home-layout';

/** Home is a 12-column grid of fixed-height rows. Positions and sizes are in cells. */
export const GRID_COLUMNS = 12;
export const GRID_ROW_PX = 52;
export const GRID_GAP_PX = 12;
export const GRID_MAX_ROW = 400;
export const GRID_MAX_HEIGHT = 24;

export interface GridRect { x: number; y: number; w: number; h: number }
export interface GridItem extends GridRect { id: string }

const SIZE_COLUMNS: Record<HomeWidgetSize, number> = { compact: 4, wide: 8, full: 12 };
const SHORTCUT_COLUMNS: Record<HomeWidgetSize, number> = { compact: 2, wide: 3, full: 4 };
const HEIGHT_ROWS: Record<HomeWidgetHeight, number> = { short: 3, medium: 4, tall: 6 };

/** Smallest a widget can be and still show its content. */
export function minimumSpan(kind: HomeWidgetKind): { w: number; h: number } {
  switch (kind) {
    case 'shortcut': return { w: 1, h: 2 };
    case 'clock': return { w: 2, h: 2 };
    case 'globe': return { w: 4, h: 5 };
    case 'graphs': return { w: 4, h: 4 };
    case 'weather': return { w: 3, h: 3 };
    default: return { w: 2, h: 3 };
  }
}

export function defaultSpan(widget: Pick<HomeWidget, 'kind' | 'size' | 'height'>): { w: number; h: number } {
  if (widget.kind === 'shortcut') {
    const w = SHORTCUT_COLUMNS[widget.size];
    return { w, h: w + 1 };
  }
  return { w: SIZE_COLUMNS[widget.size], h: HEIGHT_ROWS[widget.height] };
}

/** Closest named width and height, kept for dashboards that predate the grid. */
export function namedSize(kind: HomeWidgetKind, w: number, h: number): { size: HomeWidgetSize; height: HomeWidgetHeight } {
  const size: HomeWidgetSize = kind === 'shortcut'
    ? (w <= 2 ? 'compact' : w <= 3 ? 'wide' : 'full')
    : (w <= 5 ? 'compact' : w <= 10 ? 'wide' : 'full');
  const height: HomeWidgetHeight = h <= 3 ? 'short' : h <= 5 ? 'medium' : 'tall';
  return { size, height };
}

export function clampRect(rect: GridRect, kind: HomeWidgetKind): GridRect {
  const min = minimumSpan(kind);
  const w = Math.min(GRID_COLUMNS, Math.max(min.w, Math.round(rect.w)));
  const h = Math.min(GRID_MAX_HEIGHT, Math.max(min.h, Math.round(rect.h)));
  const x = Math.min(GRID_COLUMNS - w, Math.max(0, Math.round(rect.x)));
  const y = Math.min(GRID_MAX_ROW, Math.max(0, Math.round(rect.y)));
  return { x, y, w, h };
}

export function overlaps(a: GridRect, b: GridRect): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

function fits(rect: GridRect, placed: readonly GridItem[]): boolean {
  return rect.x >= 0 && rect.x + rect.w <= GRID_COLUMNS && placed.every((item) => !overlaps(rect, item));
}

/** First free spot scanning rows top to bottom, then columns left to right. */
function firstFit(w: number, h: number, placed: readonly GridItem[]): { x: number; y: number } {
  for (let y = 0; y <= GRID_MAX_ROW; y += 1) {
    for (let x = 0; x + w <= GRID_COLUMNS; x += 1) {
      if (fits({ x, y, w, h }, placed)) return { x, y };
    }
  }
  return { x: 0, y: bottom(placed) };
}

export function bottom(items: readonly GridRect[]): number {
  return items.reduce((max, item) => Math.max(max, item.y + item.h), 0);
}

/** Moves every item up as far as it can go, in reading order, so the grid has no holes above tiles. */
export function compact(items: readonly GridItem[], pinned?: string): GridItem[] {
  const order = [...items].sort((a, b) => a.y - b.y || a.x - b.x);
  const settled: GridItem[] = [];
  const pinnedItem = pinned === undefined ? undefined : order.find((item) => item.id === pinned);
  if (pinnedItem !== undefined) settled.push({ ...pinnedItem });
  for (const item of order) {
    if (item.id === pinned) continue;
    const next = { ...item };
    while (next.y > 0 && fits({ ...next, y: next.y - 1 }, settled)) next.y -= 1;
    while (!fits(next, settled)) next.y += 1;
    settled.push(next);
  }
  const byId = new Map(settled.map((item) => [item.id, item]));
  return items.map((item) => byId.get(item.id)!);
}

/**
 * Where every widget sits. Saved positions are kept, widgets without one are placed in
 * the first free spot in list order, and any overlap is pushed down, then everything
 * settles upward.
 */
export function resolveLayout(widgets: readonly HomeWidget[]): GridItem[] {
  const placed: GridItem[] = [];
  const pending: HomeWidget[] = [];
  for (const widget of widgets) {
    if (widget.layout === undefined) {
      pending.push(widget);
      continue;
    }
    const rect = clampRect(widget.layout, widget.kind);
    const item = { id: widget.id, ...rect };
    while (!fits(item, placed)) item.y += 1;
    placed.push(item);
  }
  for (const widget of pending) {
    const span = clampRect({ x: 0, y: 0, ...defaultSpan(widget) }, widget.kind);
    placed.push({ id: widget.id, ...firstFit(span.w, span.h, placed), w: span.w, h: span.h });
  }
  const byId = new Map(compact(placed).map((item) => [item.id, item]));
  return widgets.map((widget) => byId.get(widget.id)!);
}

/**
 * Puts `id` at `rect` exactly and makes room for it: anything it covers moves down, and the
 * rest settles upward around it.
 */
export function placeItem(items: readonly GridItem[], id: string, rect: GridRect): GridItem[] {
  const target = items.find((item) => item.id === id);
  if (target === undefined) return items.map((item) => ({ ...item }));
  const moved: GridItem = { id, ...rect };
  const others = items.filter((item) => item.id !== id).map((item) => ({ ...item }));
  // Push covered tiles below the moved one, cascading in reading order.
  const settled: GridItem[] = [moved];
  for (const item of [...others].sort((a, b) => a.y - b.y || a.x - b.x)) {
    while (!fits(item, settled)) item.y += 1;
    settled.push(item);
  }
  const compacted = compact(settled, id);
  // The moved tile may also float up into free space above where it was dropped.
  const final = compact(compacted);
  const byId = new Map(final.map((item) => [item.id, item]));
  return items.map((item) => byId.get(item.id)!);
}

/** Writes grid positions back onto widgets, keeping named sizes in step. */
export function applyLayout(widgets: readonly HomeWidget[], items: readonly GridItem[]): HomeWidget[] {
  const byId = new Map(items.map((item) => [item.id, item]));
  return widgets.map((widget) => {
    const item = byId.get(widget.id);
    if (item === undefined) return { ...widget };
    const layout = { x: item.x, y: item.y, w: item.w, h: item.h };
    return { ...widget, ...namedSize(widget.kind, item.w, item.h), layout };
  });
}

/** Pixel geometry for a cell rectangle in a container of `width` pixels. */
export function cellToPixels(rect: GridRect, width: number): { left: number; top: number; width: number; height: number } {
  const column = (width - GRID_GAP_PX * (GRID_COLUMNS - 1)) / GRID_COLUMNS;
  return {
    left: rect.x * (column + GRID_GAP_PX),
    top: rect.y * (GRID_ROW_PX + GRID_GAP_PX),
    width: rect.w * column + (rect.w - 1) * GRID_GAP_PX,
    height: rect.h * GRID_ROW_PX + (rect.h - 1) * GRID_GAP_PX,
  };
}

/** The cell a tile's top-left corner snaps to when dragged to pixel position (`left`, `top`). */
export function pixelsToCell(left: number, top: number, width: number): { x: number; y: number } {
  const column = (width - GRID_GAP_PX * (GRID_COLUMNS - 1)) / GRID_COLUMNS;
  return {
    x: Math.round(left / (column + GRID_GAP_PX)),
    y: Math.max(0, Math.round(top / (GRID_ROW_PX + GRID_GAP_PX))),
  };
}

/** Cells spanned by a pixel size, for resizing. */
export function pixelsToSpan(width: number, height: number, containerWidth: number): { w: number; h: number } {
  const column = (containerWidth - GRID_GAP_PX * (GRID_COLUMNS - 1)) / GRID_COLUMNS;
  return {
    w: Math.max(1, Math.round((width + GRID_GAP_PX) / (column + GRID_GAP_PX))),
    h: Math.max(1, Math.round((height + GRID_GAP_PX) / (GRID_ROW_PX + GRID_GAP_PX))),
  };
}

export function gridHeight(items: readonly GridRect[], extraRows = 0): number {
  const rows = bottom(items) + extraRows;
  return rows === 0 ? 0 : rows * GRID_ROW_PX + (rows - 1) * GRID_GAP_PX;
}
