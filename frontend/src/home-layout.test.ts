import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  defaultHomeWidgets,
  exportHomeTemplate,
  importHomeTemplate,
  moveHomeWidget,
  nextHomeWidgetHeight,
  nextHomeWidgetSize,
  normalizeHomeTemplates,
  normalizeHomeWidgets,
  normalizeShortcutUrl,
  parseNoteWidgetConfiguration,
  parseWeatherWidgetConfiguration,
  readHomeWidgets,
  reorderHomeWidgets,
  saveHomeWidgets,
} from './home-layout';

afterEach(() => vi.unstubAllGlobals());

describe('home layout', () => {
  it('keeps only bounded, unique, known widgets', () => {
    expect(normalizeHomeWidgets([
      { id: 'one', kind: 'note', size: 'compact', title: 'A', content: 'B' },
      { id: 'one', kind: 'clock', size: 'wide', title: 'Duplicate' },
      { id: '../bad', kind: 'clock', size: 'wide', title: 'Bad' },
    ])).toEqual([{ id: 'one', kind: 'note', size: 'compact', height: 'medium', title: 'A', content: 'B', url: '', color: '' }]);
  });

  it('ships a first-run note that can be edited without layout mode', () => {
    const notes = defaultHomeWidgets.find((widget) => widget.id === 'notes');
    expect(notes).toBeDefined();
    const parsed = parseNoteWidgetConfiguration(notes!.content);
    expect(parsed.editableOutsideLayout).toBe(true);
    expect(parsed.pages[0]?.title).toBe('Scratchpad');
    expect(parsed.pages[0]?.content).toMatch(/owner account/u);
  });

  it('preserves an intentionally empty Home', () => {
    expect(normalizeHomeWidgets([])).toEqual([]);
  });

  it('only accepts HTTP shortcuts', () => {
    expect(normalizeShortcutUrl('https://example.com/path')).toBe('https://example.com/path');
    expect(normalizeShortcutUrl('javascript:alert(1)')).toBe('');
    expect(normalizeShortcutUrl('not a URL')).toBe('');
  });

  it('keeps weather configuration bounded and defaults safely', () => {
    expect(parseWeatherWidgetConfiguration('{"location":"Denver, Colorado","unit":"celsius"}')).toEqual({ location: 'Denver, Colorado', unit: 'celsius' });
    expect(parseWeatherWidgetConfiguration('{')).toEqual({ location: '', unit: 'fahrenheit' });
    expect(normalizeHomeWidgets([{ id: 'forecast', kind: 'weather', size: 'wide', title: 'Forecast', content: '{"location":"Denver","unit":"fahrenheit"}' }])[0]?.kind).toBe('weather');
    expect(normalizeHomeWidgets([{ id: 'live', kind: 'graphs', size: 'wide', title: 'Live graphs' }, { id: 'boxes', kind: 'docker', size: 'wide', title: 'Docker' }]).map((widget) => widget.kind)).toEqual(['graphs', 'docker']);
  });

  it('reorders and resizes widgets predictably', () => {
    expect(moveHomeWidget(defaultHomeWidgets, 'host', -1).slice(0, 3).map((widget) => widget.id))
      .toEqual(['clock', 'host', 'weather']);
    expect(nextHomeWidgetSize('compact')).toBe('wide');
    expect(nextHomeWidgetSize('wide')).toBe('full');
    expect(nextHomeWidgetSize('full')).toBe('compact');
    expect(nextHomeWidgetHeight('medium')).toBe('tall');
    expect(reorderHomeWidgets(defaultHomeWidgets, 'clock', 'servers', 'after').slice(0, 4).map((widget) => widget.id))
      .toEqual(['weather', 'host', 'servers', 'clock']);
    expect(reorderHomeWidgets(defaultHomeWidgets, 'servers', 'clock', 'before').slice(0, 4).map((widget) => widget.id))
      .toEqual(['servers', 'clock', 'weather', 'host']);
  });

  it('round trips a local layout', () => {
    const values = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    });
    saveHomeWidgets([{ id: 'link', kind: 'shortcut', size: 'compact', height: 'short', title: 'Docs', content: '', url: 'https://example.com', color: '#ff8800' }]);
    expect(readHomeWidgets()).toEqual([{ id: 'link', kind: 'shortcut', size: 'compact', height: 'short', title: 'Docs', content: '', url: 'https://example.com', color: '#ff8800' }]);
  });

  it('migrates legacy notes and bounds shared Home templates', () => {
    expect(parseNoteWidgetConfiguration('keep this')).toMatchObject({ pages: [{ content: 'keep this' }] });
    const templates = normalizeHomeTemplates([{ id: 'work', name: 'Work', accent: '#ff8800', widgets: [] }]);
    expect(templates).toEqual([{ id: 'work', name: 'Work', accent: '#ff8800', widgets: [] }]);
    const imported = importHomeTemplate(exportHomeTemplate(templates[0]!));
    expect(imported).toMatchObject({ name: 'Work', accent: '#ff8800', widgets: [] });
    expect(imported.id).not.toBe('work');
  });
});
