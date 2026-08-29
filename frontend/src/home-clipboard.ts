import {
  collectHomeWidgets,
  MAX_WIDGETS_PER_HOME,
  normalizeShortcutUrl,
  type HomeTemplate,
  type HomeWidget,
  type HomeWidgetKind,
} from './home-layout';

const WIDGET_CLIPBOARD_STORAGE_KEY = 'helix.home.widget-clipboard';
const HOME_WIDGET_CLIPBOARD_FORMAT = 'helix-home-widgets';
const MAX_CLIPBOARD_CHARACTERS = 70_000;
const MAX_TOTAL_WIDGETS = 64;

export function newHomeWidgetId(kind: HomeWidgetKind, salt = ''): string {
  return `${kind}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}${salt.length > 0 ? `-${salt}` : ''}`;
}

export function cloneHomeWidgets(widgets: readonly HomeWidget[]): HomeWidget[] {
  const stamp = Date.now().toString(36);
  return widgets.map((widget, index) => ({
    ...widget,
    id: newHomeWidgetId(widget.kind, `${stamp}${index.toString(36)}`),
  }));
}

function totalHomeWidgetCount(templates: readonly HomeTemplate[]): number {
  return templates.reduce((total, template) => total + template.widgets.length, 0);
}

export function mergeHomarrShortcuts(
  existing: readonly HomeWidget[],
  incoming: readonly HomeWidget[],
): HomeWidget[] {
  const previous = new Map<string, HomeWidget>();
  for (const widget of existing) {
    if (widget.kind === 'shortcut' && widget.url.length > 0) previous.set(widget.url, widget);
  }
  const merged = incoming.map((widget) => {
    const kept = previous.get(widget.url);
    if (kept === undefined) return { ...widget };
    return {
      ...kept,
      icon: kept.icon.length > 0 ? kept.icon : widget.icon,
    };
  });
  const extras = existing.filter((widget) => widget.kind !== 'shortcut');
  const room = Math.max(0, MAX_WIDGETS_PER_HOME - extras.length);
  return [...merged.slice(0, room), ...extras];
}

export function exportHomeWidgetsClipboard(widgets: readonly HomeWidget[]): string {
  return JSON.stringify({
    format: HOME_WIDGET_CLIPBOARD_FORMAT,
    version: 1,
    widgets: collectHomeWidgets(widgets).map((widget) => ({
      kind: widget.kind,
      size: widget.size,
      height: widget.height,
      title: widget.title,
      content: widget.content,
      url: widget.url,
      color: widget.color,
      icon: widget.icon,
    })),
  });
}

export function parseHomeWidgetsClipboard(value: string): HomeWidget[] {
  const raw = value.trim();
  if (raw.length === 0 || raw.length > MAX_CLIPBOARD_CHARACTERS) return [];
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return [];
    const record = parsed as Record<string, unknown>;
    if (record.version !== 1) return [];
    let items: unknown = null;
    if (record.format === HOME_WIDGET_CLIPBOARD_FORMAT) {
      items = record.widgets;
    } else if (record.format === 'helix-home-template') {
      const template = record.template;
      if (typeof template === 'object' && template !== null && !Array.isArray(template)) {
        items = (template as Record<string, unknown>).widgets;
      }
    } else {
      return [];
    }
    if (!Array.isArray(items)) return [];
    const withIds = items.map((item, index) => {
      if (typeof item !== 'object' || item === null || Array.isArray(item)) return item;
      return { ...(item as Record<string, unknown>), id: `clip${index}` };
    });
    return collectHomeWidgets(withIds).map((widget) => (
      widget.kind === 'shortcut'
        ? { ...widget, url: normalizeShortcutUrl(widget.url), icon: normalizeShortcutUrl(widget.icon) }
        : widget
    ));
  } catch {
    return [];
  }
}

export function saveWidgetClipboard(widgets: readonly HomeWidget[]): void {
  try {
    globalThis.localStorage?.setItem(WIDGET_CLIPBOARD_STORAGE_KEY, exportHomeWidgetsClipboard(widgets));
  } catch {
    // Copy still works in memory if browser storage is unavailable.
  }
}

export function readWidgetClipboard(): HomeWidget[] {
  try {
    const stored = globalThis.localStorage?.getItem(WIDGET_CLIPBOARD_STORAGE_KEY);
    return stored === null || stored === undefined ? [] : parseHomeWidgetsClipboard(stored);
  } catch {
    return [];
  }
}

export function pasteHomeWidgets(
  templates: readonly HomeTemplate[],
  homeId: string,
  incoming: readonly HomeWidget[],
  insertAfterId: string | null = null,
): { templates: HomeTemplate[]; pasted: HomeWidget[] } | { error: string } {
  const target = templates.find((template) => template.id === homeId);
  if (target === undefined) return { error: 'That Home is no longer here.' };
  const clones = cloneHomeWidgets(incoming);
  if (clones.length === 0) return { error: 'Copy a widget first.' };
  const roomOnHome = MAX_WIDGETS_PER_HOME - target.widgets.length;
  const roomOverall = MAX_TOTAL_WIDGETS - totalHomeWidgetCount(templates);
  const room = Math.min(roomOnHome, roomOverall, clones.length);
  if (room <= 0) {
    if (roomOnHome <= 0) return { error: `${target.name} already has ${MAX_WIDGETS_PER_HOME} widgets.` };
    return { error: 'Homes can hold 64 widgets in total. Remove some, then paste again.' };
  }
  const pasted = clones.slice(0, room);
  const nextWidgets = target.widgets.map((widget) => ({ ...widget }));
  const insertAt = insertAfterId === null ? nextWidgets.length : nextWidgets.findIndex((widget) => widget.id === insertAfterId) + 1;
  const index = insertAt <= 0 ? nextWidgets.length : insertAt;
  nextWidgets.splice(index, 0, ...pasted);
  return {
    templates: templates.map((template) => template.id === homeId ? { ...template, widgets: nextWidgets } : template),
    pasted,
  };
}
