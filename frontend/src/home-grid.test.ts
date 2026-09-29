import { describe, expect, it } from 'vitest';
import { applyLayout, cellToPixels, clampRect, compact, defaultSpan, overlaps, pixelsToCell, pixelsToSpan, placeItem, resolveLayout, type GridItem } from './home-grid';
import { collectHomeWidgets, type HomeWidget } from './home-layout';

const widget = (id: string, extra: Partial<HomeWidget> = {}): HomeWidget => ({
  id, kind: 'host', size: 'compact', height: 'medium', title: id, content: '', url: '', color: '', icon: '', ...extra,
});

function noOverlaps(items: readonly GridItem[]): boolean {
  return items.every((a, i) => items.every((b, j) => i === j || !overlaps(a, b)));
}

describe('home grid', () => {
  it('places unpositioned widgets in reading order, the same way every time', () => {
    const widgets = [widget('a'), widget('b', { size: 'wide' }), widget('c'), widget('d', { size: 'full', height: 'short' })];
    const first = resolveLayout(widgets);
    expect(first).toEqual(resolveLayout(widgets));
    expect(first.map(({ id, x, y, w, h }) => [id, x, y, w, h])).toEqual([
      ['a', 0, 0, 4, 4], ['b', 4, 0, 8, 4], ['c', 0, 4, 4, 4], ['d', 0, 8, 12, 3],
    ]);
    expect(noOverlaps(first)).toBe(true);
  });

  it('lets a tile sit under a short neighbour beside a tall one', () => {
    // A tall tile on the left and a short one on the right leave room under the short one.
    const items: GridItem[] = [
      { id: 'tall', x: 0, y: 0, w: 4, h: 8 },
      { id: 'short', x: 4, y: 0, w: 8, h: 3 },
      { id: 'moving', x: 0, y: 8, w: 8, h: 3 },
    ];
    const next = placeItem(items, 'moving', { x: 4, y: 3, w: 8, h: 3 });
    expect(next.find((item) => item.id === 'moving')).toMatchObject({ x: 4, y: 3 });
    expect(next.find((item) => item.id === 'tall')).toMatchObject({ x: 0, y: 0, h: 8 });
    expect(noOverlaps(next)).toBe(true);
  });

  it('pushes covered tiles down and lets everything settle upward', () => {
    const items: GridItem[] = [
      { id: 'a', x: 0, y: 0, w: 6, h: 4 },
      { id: 'b', x: 6, y: 0, w: 6, h: 4 },
      { id: 'c', x: 0, y: 4, w: 12, h: 3 },
    ];
    const next = placeItem(items, 'c', { x: 0, y: 0, w: 12, h: 3 });
    expect(next.find((item) => item.id === 'c')).toMatchObject({ y: 0 });
    expect(next.find((item) => item.id === 'a')).toMatchObject({ y: 3 });
    expect(next.find((item) => item.id === 'b')).toMatchObject({ y: 3 });
    expect(noOverlaps(next)).toBe(true);
  });

  it('floats a dropped tile up into free space and never leaves holes above tiles', () => {
    const items: GridItem[] = [{ id: 'a', x: 0, y: 0, w: 4, h: 4 }, { id: 'b', x: 4, y: 0, w: 4, h: 4 }];
    const next = placeItem(items, 'b', { x: 8, y: 9, w: 4, h: 4 });
    expect(next.find((item) => item.id === 'b')).toMatchObject({ x: 8, y: 0 });
    expect(compact([{ id: 'z', x: 0, y: 7, w: 4, h: 2 }])[0]).toMatchObject({ y: 0 });
  });

  it('resolves overlapping saved positions without losing a tile', () => {
    const widgets = [
      widget('a', { layout: { x: 0, y: 0, w: 6, h: 4 } }),
      widget('b', { layout: { x: 2, y: 1, w: 6, h: 4 } }),
      widget('c'),
    ];
    const items = resolveLayout(widgets);
    expect(items).toHaveLength(3);
    expect(noOverlaps(items)).toBe(true);
  });

  it('keeps tiles inside the grid and above their minimum size', () => {
    expect(clampRect({ x: 10, y: -3, w: 6, h: 1 }, 'globe')).toEqual({ x: 6, y: 0, w: 6, h: 5 });
    expect(clampRect({ x: 0, y: 0, w: 20, h: 99 }, 'note')).toEqual({ x: 0, y: 0, w: 12, h: 24 });
    expect(defaultSpan({ kind: 'shortcut', size: 'compact', height: 'short' })).toEqual({ w: 2, h: 3 });
  });

  it('writes positions back with matching named sizes and keeps them through a save', () => {
    const widgets = [widget('a'), widget('b')];
    const placed = applyLayout(widgets, placeItem(resolveLayout(widgets), 'b', { x: 0, y: 0, w: 12, h: 6 }));
    expect(placed.find((item) => item.id === 'b')).toMatchObject({ size: 'full', height: 'tall', layout: { x: 0, y: 0, w: 12, h: 6 } });
    const reloaded = collectHomeWidgets(JSON.parse(JSON.stringify(placed)));
    expect(reloaded.map((item) => item.layout)).toEqual(placed.map((item) => item.layout));
    expect(collectHomeWidgets([{ ...widget('x'), layout: { x: 10, y: 0, w: 4, h: 4 } }])[0]?.layout).toBeUndefined();
  });

  it('converts between cells and pixels consistently', () => {
    const width = 1200;
    const box = cellToPixels({ x: 3, y: 2, w: 4, h: 3 }, width);
    expect(pixelsToCell(box.left + 5, box.top - 5, width)).toEqual({ x: 3, y: 2 });
    expect(pixelsToSpan(box.width, box.height, width)).toEqual({ w: 4, h: 3 });
  });
});
