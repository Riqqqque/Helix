export type HomeWidgetKind = 'clock' | 'host' | 'servers' | 'storage' | 'weather' | 'note' | 'shortcut' | 'graphs' | 'docker';
export type HomeWidgetSize = 'compact' | 'wide' | 'full';
export type HomeWidgetHeight = 'short' | 'medium' | 'tall';

export interface HomeWidget {
  id: string;
  kind: HomeWidgetKind;
  size: HomeWidgetSize;
  height: HomeWidgetHeight;
  title: string;
  content: string;
  url: string;
  color: string;
}

export interface HomeTemplate {
  id: string;
  name: string;
  accent: string;
  widgets: HomeWidget[];
}

export interface NotePage {
  id: string;
  title: string;
  content: string;
}

export interface NoteWidgetConfiguration {
  version: 1;
  activePageId: string;
  editableOutsideLayout: boolean;
  pages: NotePage[];
}

const STORAGE_KEY = 'helix.home.widgets';
const TEMPLATES_STORAGE_KEY = 'helix.home.templates';
const ACTIVE_HOME_STORAGE_KEY = 'helix.home.active-template';
const kinds = new Set<HomeWidgetKind>(['clock', 'host', 'servers', 'storage', 'weather', 'note', 'shortcut', 'graphs', 'docker']);
const sizes = new Set<HomeWidgetSize>(['compact', 'wide', 'full']);
const heights = new Set<HomeWidgetHeight>(['short', 'medium', 'tall']);
const MAX_HOME_TEMPLATES = 8;
const MAX_TOTAL_WIDGETS = 64;
const MAX_NOTE_PAGES = 8;
const MAX_NOTE_TEXT_CHARACTERS = 7_000;
const defaultAccent = '#d7f64d';

export const defaultHomeWidgets: ReadonlyArray<HomeWidget> = [
  { id: 'clock', kind: 'clock', size: 'compact', height: 'medium', title: 'Right now', content: '', url: '', color: '' },
  { id: 'weather', kind: 'weather', size: 'wide', height: 'medium', title: 'Weather', content: '', url: '', color: '' },
  { id: 'host', kind: 'host', size: 'wide', height: 'medium', title: 'Host pulse', content: '', url: '', color: '' },
  { id: 'servers', kind: 'servers', size: 'wide', height: 'medium', title: 'Servers', content: '', url: '', color: '' },
  { id: 'storage', kind: 'storage', size: 'wide', height: 'medium', title: 'Storage', content: '', url: '', color: '' },
  {
    id: 'notes',
    kind: 'note',
    size: 'compact',
    height: 'medium',
    title: 'Notes',
    content: '{"version":1,"activePageId":"page-1","editableOutsideLayout":true,"pages":[{"id":"page-1","title":"Scratchpad","content":"This note stays with your owner account. Use it for ISP details, port forwards, or weekend plans. Keep passwords in a password manager."}]}',
    url: '',
    color: '',
  },
];

export const defaultHomeTemplates: ReadonlyArray<HomeTemplate> = [{
  id: 'home-main',
  name: 'Main',
  accent: defaultAccent,
  widgets: defaultHomeWidgets.map((widget) => ({ ...widget })),
}];

export interface WeatherWidgetConfiguration {
  location: string;
  unit: 'celsius' | 'fahrenheit';
}

export function parseWeatherWidgetConfiguration(value: unknown): WeatherWidgetConfiguration {
  try {
    const record = typeof value === 'string' ? JSON.parse(value) as unknown : value;
    if (typeof record !== 'object' || record === null || Array.isArray(record)) throw new Error();
    const candidate = record as Record<string, unknown>;
    const location = cleanText(candidate.location, 120).trim();
    const unit = candidate.unit === 'celsius' ? 'celsius' : 'fahrenheit';
    if (Array.from(location).some((character) => /\p{Cc}/u.test(character))) throw new Error();
    return { location, unit };
  } catch {
    return { location: '', unit: 'fahrenheit' };
  }
}

export function serializeWeatherWidgetConfiguration(configuration: WeatherWidgetConfiguration): string {
  return JSON.stringify(parseWeatherWidgetConfiguration(configuration));
}

function cleanText(value: unknown, maximum: number): string {
  return typeof value === 'string' ? value.slice(0, maximum) : '';
}

export function normalizeAccent(value: unknown): string {
  const candidate = cleanText(value, 7).toLowerCase();
  return /^#[0-9a-f]{6}$/u.test(candidate) ? candidate : defaultAccent;
}

function normalizeWidgetColor(value: unknown): string {
  const candidate = cleanText(value, 7).toLowerCase();
  return candidate.length === 0 || /^#[0-9a-f]{6}$/u.test(candidate) ? candidate : '';
}

export function normalizeShortcutUrl(value: unknown): string {
  const raw = cleanText(value, 2_048).trim();
  if (raw.length === 0) return '';
  try {
    const parsed = new URL(raw);
    return parsed.protocol === 'http:' || parsed.protocol === 'https:' ? parsed.href : '';
  } catch {
    return '';
  }
}

export function normalizeHomeWidgets(value: unknown): HomeWidget[] {
  if (!Array.isArray(value)) return defaultHomeWidgets.map((widget) => ({ ...widget }));
  if (value.length === 0) return [];
  const ids = new Set<string>();
  const widgets: HomeWidget[] = [];

  for (const item of value.slice(0, 32)) {
    if (typeof item !== 'object' || item === null) continue;
    const record = item as Record<string, unknown>;
    const id = cleanText(record.id, 96);
    const kind = record.kind;
    const size = record.size;
    if (
      !/^[a-zA-Z0-9_-]+$/u.test(id) ||
      ids.has(id) ||
      typeof kind !== 'string' ||
      !kinds.has(kind as HomeWidgetKind) ||
      typeof size !== 'string' ||
      !sizes.has(size as HomeWidgetSize)
    ) continue;
    ids.add(id);
    widgets.push({
      id,
      kind: kind as HomeWidgetKind,
      size: size as HomeWidgetSize,
      height: typeof record.height === 'string' && heights.has(record.height as HomeWidgetHeight)
        ? record.height as HomeWidgetHeight
        : 'medium',
      title: cleanText(record.title, 80).trim() || kind,
      content: cleanText(record.content, 8_000),
      url: kind === 'shortcut' ? cleanText(record.url, 2_048) : '',
      color: normalizeWidgetColor(record.color),
    });
  }

  return widgets.length === 0 ? defaultHomeWidgets.map((widget) => ({ ...widget })) : widgets;
}

function cloneDefaultTemplate(widgets: readonly HomeWidget[] = defaultHomeWidgets): HomeTemplate {
  return {
    id: 'home-main',
    name: 'Main',
    accent: defaultAccent,
    widgets: widgets.map((widget) => ({ ...widget })),
  };
}

export function normalizeHomeTemplates(
  value: unknown,
  fallbackWidgets: readonly HomeWidget[] = defaultHomeWidgets,
): HomeTemplate[] {
  if (!Array.isArray(value)) return [cloneDefaultTemplate(fallbackWidgets)];
  const ids = new Set<string>();
  const templates: HomeTemplate[] = [];
  let totalWidgets = 0;

  for (const item of value.slice(0, MAX_HOME_TEMPLATES)) {
    if (typeof item !== 'object' || item === null || Array.isArray(item)) continue;
    const record = item as Record<string, unknown>;
    const id = cleanText(record.id, 96);
    const name = cleanText(record.name, 48).trim();
    if (!/^[a-zA-Z0-9_-]+$/u.test(id) || ids.has(id) || name.length === 0) continue;
    const widgets = Array.isArray(record.widgets) && record.widgets.length === 0
      ? []
      : normalizeHomeWidgets(record.widgets);
    const remaining = MAX_TOTAL_WIDGETS - totalWidgets;
    if (remaining < 0) break;
    const boundedWidgets = widgets.slice(0, remaining);
    ids.add(id);
    templates.push({
      id,
      name,
      accent: normalizeAccent(record.accent),
      widgets: boundedWidgets,
    });
    totalWidgets += boundedWidgets.length;
    if (totalWidgets >= MAX_TOTAL_WIDGETS) break;
  }

  return templates.length === 0 ? [cloneDefaultTemplate(fallbackWidgets)] : templates;
}

export function normalizeActiveHomeId(value: unknown, templates: readonly HomeTemplate[]): string {
  const candidate = cleanText(value, 96);
  return templates.some((template) => template.id === candidate)
    ? candidate
    : templates[0]?.id ?? 'home-main';
}

export function parseNoteWidgetConfiguration(value: unknown): NoteWidgetConfiguration {
  const legacy = cleanText(value, 8_000);
  try {
    const parsed = typeof value === 'string' ? JSON.parse(value) as unknown : value;
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) throw new Error();
    const record = parsed as Record<string, unknown>;
    if (record.version !== 1 || !Array.isArray(record.pages)) throw new Error();
    const ids = new Set<string>();
    const pages: NotePage[] = [];
    let remainingCharacters = MAX_NOTE_TEXT_CHARACTERS;
    for (const item of record.pages.slice(0, MAX_NOTE_PAGES)) {
      if (typeof item !== 'object' || item === null || Array.isArray(item)) continue;
      const page = item as Record<string, unknown>;
      const id = cleanText(page.id, 96);
      const title = cleanText(page.title, 48).trim();
      if (!/^[a-zA-Z0-9_-]+$/u.test(id) || ids.has(id) || title.length === 0) continue;
      const content = cleanText(page.content, remainingCharacters);
      remainingCharacters -= Array.from(content).length;
      ids.add(id);
      pages.push({ id, title, content });
      if (remainingCharacters <= 0) break;
    }
    if (pages.length === 0) throw new Error();
    const activePageId = pages.some((page) => page.id === record.activePageId)
      ? String(record.activePageId)
      : pages[0]!.id;
    return {
      version: 1,
      activePageId,
      editableOutsideLayout: record.editableOutsideLayout === true,
      pages,
    };
  } catch {
    return {
      version: 1,
      activePageId: 'page-1',
      editableOutsideLayout: false,
      pages: [{ id: 'page-1', title: 'Page 1', content: legacy.slice(0, MAX_NOTE_TEXT_CHARACTERS) }],
    };
  }
}

export function serializeNoteWidgetConfiguration(configuration: NoteWidgetConfiguration): string {
  return JSON.stringify(parseNoteWidgetConfiguration(configuration));
}

export function readHomeWidgets(): HomeWidget[] {
  try {
    const stored = globalThis.localStorage?.getItem(STORAGE_KEY);
    return normalizeHomeWidgets(stored === null || stored === undefined ? null : JSON.parse(stored));
  } catch {
    return defaultHomeWidgets.map((widget) => ({ ...widget }));
  }
}

const HOME_FOCUS_KEY = 'helix.home.focus';

export function readHomeFocus(): boolean {
  try {
    return globalThis.localStorage?.getItem(HOME_FOCUS_KEY) === '1';
  } catch {
    return false;
  }
}

export function saveHomeFocus(value: boolean): void {
  try {
    if (value) globalThis.localStorage?.setItem(HOME_FOCUS_KEY, '1');
    else globalThis.localStorage?.removeItem(HOME_FOCUS_KEY);
  } catch {
    // Focus mode still works in memory if browser storage is unavailable.
  }
}

export function saveHomeWidgets(widgets: readonly HomeWidget[]): void {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify(normalizeHomeWidgets(widgets)));
  } catch {
    // Local customization is optional; the live dashboard keeps working without storage.
  }
}

export function readHomeTemplates(): HomeTemplate[] {
  const legacy = readHomeWidgets();
  try {
    const stored = globalThis.localStorage?.getItem(TEMPLATES_STORAGE_KEY);
    return normalizeHomeTemplates(stored === null || stored === undefined ? null : JSON.parse(stored), legacy);
  } catch {
    return [cloneDefaultTemplate(legacy)];
  }
}

export function saveHomeTemplates(templates: readonly HomeTemplate[]): void {
  try {
    globalThis.localStorage?.setItem(TEMPLATES_STORAGE_KEY, JSON.stringify(normalizeHomeTemplates(templates)));
  } catch {
    // The server-backed preference remains authoritative when browser storage is unavailable.
  }
}

export function readActiveHomeId(templates: readonly HomeTemplate[]): string {
  try {
    return normalizeActiveHomeId(globalThis.localStorage?.getItem(ACTIVE_HOME_STORAGE_KEY), templates);
  } catch {
    return normalizeActiveHomeId(null, templates);
  }
}

export function saveActiveHomeId(value: string, templates: readonly HomeTemplate[]): void {
  try {
    globalThis.localStorage?.setItem(ACTIVE_HOME_STORAGE_KEY, normalizeActiveHomeId(value, templates));
  } catch {
    // Active selection is still retained in memory and synced through Helix.
  }
}

export function moveHomeWidget(
  widgets: readonly HomeWidget[],
  id: string,
  offset: -1 | 1,
): HomeWidget[] {
  const next = widgets.map((widget) => ({ ...widget }));
  const index = next.findIndex((widget) => widget.id === id);
  const destination = index + offset;
  if (index < 0 || destination < 0 || destination >= next.length) return next;
  [next[index], next[destination]] = [next[destination]!, next[index]!];
  return next;
}

export function nextHomeWidgetSize(size: HomeWidgetSize): HomeWidgetSize {
  if (size === 'compact') return 'wide';
  if (size === 'wide') return 'full';
  return 'compact';
}

export function nextHomeWidgetHeight(height: HomeWidgetHeight): HomeWidgetHeight {
  if (height === 'short') return 'medium';
  if (height === 'medium') return 'tall';
  return 'short';
}

export function reorderHomeWidgets(
  widgets: readonly HomeWidget[],
  sourceId: string,
  targetId: string,
  placement: 'before' | 'after' = 'before',
): HomeWidget[] {
  const next = widgets.map((widget) => ({ ...widget }));
  const source = next.findIndex((widget) => widget.id === sourceId);
  if (source < 0 || sourceId === targetId) return next;
  const [moved] = next.splice(source, 1);
  const target = next.findIndex((widget) => widget.id === targetId);
  if (moved === undefined || target < 0) return widgets.map((widget) => ({ ...widget }));
  next.splice(target + (placement === 'after' ? 1 : 0), 0, moved);
  return next;
}

export function exportHomeTemplate(template: HomeTemplate): string {
  return JSON.stringify({ format: 'helix-home-template', version: 1, template: normalizeHomeTemplates([template])[0] }, null, 2);
}

export function importHomeTemplate(value: string): HomeTemplate {
  const parsed = JSON.parse(value) as unknown;
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) throw new Error('Template file is invalid.');
  const record = parsed as Record<string, unknown>;
  if (record.format !== 'helix-home-template' || record.version !== 1) throw new Error('This is not a supported Helix Home template.');
  if (typeof record.template !== 'object' || record.template === null || Array.isArray(record.template)) throw new Error('Template file is invalid.');
  const rawTemplate = record.template as Record<string, unknown>;
  if (
    typeof rawTemplate.id !== 'string' || !/^[a-zA-Z0-9_-]{1,96}$/u.test(rawTemplate.id) ||
    typeof rawTemplate.name !== 'string' || rawTemplate.name.trim().length === 0 ||
    typeof rawTemplate.accent !== 'string' || !/^#[0-9a-fA-F]{6}$/u.test(rawTemplate.accent) ||
    !Array.isArray(rawTemplate.widgets) || rawTemplate.widgets.length > 32
  ) throw new Error('Template file is invalid.');
  const templates = normalizeHomeTemplates([record.template], []);
  const template = templates[0];
  if (template === undefined || template.widgets.length !== rawTemplate.widgets.length) throw new Error('Template file is invalid.');
  return {
    ...template,
    id: `home-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`,
    widgets: template.widgets.map((widget, index) => ({ ...widget, id: `${widget.kind}-${Date.now().toString(36)}-${index.toString(36)}` })),
  };
}
