import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  defaultHomeWidgets,
  exportHomeTemplate,
  homeShortcutUrls,
  importHomeTemplate,
  moveHomeWidget,
  homarrShortcutSize,
  newHomarrShortcuts,
  nextHomeWidgetHeight,
  nextHomeWidgetSize,
  normalizeHomeTemplates,
  normalizeHomeWidgets,
  normalizeShortcutUrl,
  parseNoteWidgetConfiguration,
  parseWeatherWidgetConfiguration,
  readHomeWidgets,
  replaceHomarrHome,
  reorderHomeWidgets,
  saveHomeWidgets,
  HOMARR_HOME_ID,
  cloneHomeWidgets,
  exportHomeWidgetsClipboard,
  mergeHomarrShortcuts,
  parseHomeWidgetsClipboard,
  pasteHomeWidgets,
  type HomeWidget,
} from './home-layout';

afterEach(() => vi.unstubAllGlobals());

describe('home layout', () => {
  it('keeps only bounded, unique, known widgets', () => {
    expect(normalizeHomeWidgets([
      { id: 'one', kind: 'note', size: 'compact', title: 'A', content: 'B' },
      { id: 'one', kind: 'clock', size: 'wide', title: 'Duplicate' },
      { id: '../bad', kind: 'clock', size: 'wide', title: 'Bad' },
    ])).toEqual([{ id: 'one', kind: 'note', size: 'compact', height: 'medium', title: 'A', content: 'B', url: '', color: '', icon: '' }]);
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
    expect(normalizeHomeWidgets([{ id: 'live', kind: 'graphs', size: 'wide', title: 'Live graphs' }, { id: 'boxes', kind: 'docker', size: 'wide', title: 'Docker' }, { id: 'ext', kind: 'strand', size: 'wide', title: 'System Health', url: 'b893d568-327d-4b6e-b0b6-0b7a58e0c852' }]).map((widget) => widget.kind)).toEqual(['graphs', 'docker', 'strand']);
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
    saveHomeWidgets([{ id: 'link', kind: 'shortcut', size: 'compact', height: 'short', title: 'Docs', content: '', url: 'https://example.com', color: '#ff8800', icon: 'https://example.test/docs.png' }]);
    expect(readHomeWidgets()).toEqual([{ id: 'link', kind: 'shortcut', size: 'compact', height: 'short', title: 'Docs', content: '', url: 'https://example.com', color: '#ff8800', icon: 'https://example.test/docs.png' }]);
  });

  it('migrates legacy notes and bounds shared Home templates', () => {
    expect(parseNoteWidgetConfiguration('keep this')).toMatchObject({ pages: [{ content: 'keep this' }] });
    const templates = normalizeHomeTemplates([{ id: 'work', name: 'Work', accent: '#ff8800', widgets: [] }]);
    expect(templates).toEqual([{ id: 'work', name: 'Work', accent: '#ff8800', widgets: [] }]);
    const imported = importHomeTemplate(exportHomeTemplate(templates[0]!));
    expect(imported).toMatchObject({ name: 'Work', accent: '#ff8800', widgets: [] });
    expect(imported.id).not.toBe('work');
  });

  it('imports Homarr shortcuts that are not already on Home', () => {
    const existing: HomeWidget[] = [
      { id: 'clock', kind: 'clock', size: 'compact', height: 'medium', title: 'Now', content: '', url: '', color: '', icon: '' },
      { id: 'plex', kind: 'shortcut', size: 'compact', height: 'medium', title: 'Plex', content: '', url: 'http://192.168.1.10:32400/web', color: '', icon: '' },
    ];
    expect(homeShortcutUrls(existing)).toEqual(new Set(['http://192.168.1.10:32400/web']));
    expect(newHomarrShortcuts([
      { url: 'http://192.168.1.10:32400/web' },
      { url: 'http://192.168.1.10:7878' },
      { url: 'http://192.168.1.10:7878' },
      { url: '' },
    ], existing)).toEqual([{ url: 'http://192.168.1.10:7878' }]);
  });

  it('places Homarr shortcuts on a dedicated Homarr Home in catalog order', () => {
    const templates = [
      { id: 'home-main', name: 'Main', accent: '#d7f64d', widgets: defaultHomeWidgets.map((widget) => ({ ...widget })) },
    ];
    const result = replaceHomarrHome(templates, [
      { id: 'shortcut-homarr-0', kind: 'shortcut', size: 'compact', height: 'short', title: 'Radarr', content: '', url: 'http://192.168.1.10:7878', color: '', icon: 'https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/png/radarr.png' },
      { id: 'shortcut-homarr-1', kind: 'shortcut', size: 'wide', height: 'short', title: 'Plex', content: '', url: 'http://192.168.1.10:32400/web', color: '', icon: 'https://example.test/plex.png' },
    ]);
    expect('templates' in result).toBe(true);
    if (!('templates' in result)) return;
    expect(result.activeHomeId).toBe(HOMARR_HOME_ID);
    expect(result.templates.map((template) => template.id)).toEqual(['home-main', HOMARR_HOME_ID]);
    expect(result.templates[1]?.widgets.map((widget) => widget.title)).toEqual(['Radarr', 'Plex']);
    expect(homarrShortcutSize(1)).toBe('compact');
    expect(homarrShortcutSize(4)).toBe('wide');
    expect(homarrShortcutSize(8)).toBe('full');
  });

  it('keeps edited Homarr shortcuts when the same apps are imported again', () => {
    const existing: HomeWidget[] = [
      { id: 'shortcut-plex', kind: 'shortcut', size: 'wide', height: 'medium', title: 'Living room Plex', content: '', url: 'http://192.168.1.10:32400/web', color: '#ff8800', icon: 'https://example.test/plex.png' },
      { id: 'clock', kind: 'clock', size: 'compact', height: 'medium', title: 'Right now', content: '', url: '', color: '', icon: '' },
    ];
    const incoming: HomeWidget[] = [
      { id: 'shortcut-new-radarr', kind: 'shortcut', size: 'compact', height: 'short', title: 'Radarr', content: '', url: 'http://192.168.1.10:7878', color: '', icon: '' },
      { id: 'shortcut-new-plex', kind: 'shortcut', size: 'compact', height: 'short', title: 'Plex', content: '', url: 'http://192.168.1.10:32400/web', color: '', icon: 'https://cdn.example/plex.png' },
    ];
    const merged = mergeHomarrShortcuts(existing, incoming);
    expect(merged.map((widget) => widget.id)).toEqual(['shortcut-new-radarr', 'shortcut-plex', 'clock']);
    expect(merged[1]).toMatchObject({ title: 'Living room Plex', size: 'wide', height: 'medium', color: '#ff8800', icon: 'https://example.test/plex.png' });
    expect(mergeHomarrShortcuts(existing, [incoming[0]!]).map((widget) => widget.id)).toEqual(['shortcut-new-radarr', 'clock']);
  });

  it('copies widgets onto another Home with new ids and rejects script URLs', () => {
    const plex: HomeWidget = { id: 'shortcut-plex', kind: 'shortcut', size: 'compact', height: 'short', title: 'Plex', content: '', url: 'http://192.168.1.10:32400/web', color: '', icon: '' };
    const payload = exportHomeWidgetsClipboard([plex]);
    expect(parseHomeWidgetsClipboard(payload)[0]).toMatchObject({ kind: 'shortcut', title: 'Plex', url: 'http://192.168.1.10:32400/web' });
    expect(parseHomeWidgetsClipboard('{"format":"helix-home-widgets","version":1,"widgets":[{"kind":"shortcut","size":"compact","title":"Nope","url":"javascript:alert(1)"}]}')[0]?.url).toBe('');
    const clones = cloneHomeWidgets([plex]);
    expect(clones[0]?.id).not.toBe('shortcut-plex');
    expect(clones[0]?.url).toBe(plex.url);
    const templates = [
      { id: 'home-main', name: 'Main', accent: '#d7f64d', widgets: defaultHomeWidgets.map((widget) => ({ ...widget })) },
      { id: HOMARR_HOME_ID, name: 'Homarr', accent: '#d7f64d', widgets: [plex] },
    ];
    const result = pasteHomeWidgets(templates, 'home-main', [plex], 'clock');
    expect('templates' in result).toBe(true);
    if (!('templates' in result)) return;
    expect(result.pasted).toHaveLength(1);
    expect(result.templates[0]?.widgets[1]).toMatchObject({ kind: 'shortcut', title: 'Plex', url: plex.url });
    expect(result.templates[0]?.widgets[1]?.id).not.toBe('shortcut-plex');
    expect(result.templates[1]?.widgets).toHaveLength(1);
  });

  it('refuses to paste past the per-Home cap', () => {
    const full = Array.from({ length: 32 }, (_, index) => ({
      id: `clock-${index}`,
      kind: 'clock' as const,
      size: 'compact' as const,
      height: 'medium' as const,
      title: 'Clock',
      content: '',
      url: '',
      color: '',
      icon: '',
    }));
    const result = pasteHomeWidgets(
      [{ id: 'home-main', name: 'Main', accent: '#d7f64d', widgets: full }],
      'home-main',
      [{ id: 'extra', kind: 'clock', size: 'compact', height: 'medium', title: 'Extra', content: '', url: '', color: '', icon: '' }],
    );
    expect(result).toEqual({ error: 'Main already has 32 widgets.' });
  });
});
