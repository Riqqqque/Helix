import { useEffect, useMemo, useState } from 'preact/hooks';
import type { HostInventory, ManagedServer } from './control-api';
import { calculatePercent, formatBytes, formatDuration, formatPercent } from './format';
import {
  exportHomeTemplate,
  homarrShortcutSize,
  homeShortcutUrls,
  HOMARR_HOME_ID,
  importHomeTemplate,
  moveHomeWidget,
  newHomarrShortcuts,
  nextHomeWidgetHeight,
  nextHomeWidgetSize,
  normalizeShortcutUrl,
  parseNoteWidgetConfiguration,
  parseWeatherWidgetConfiguration,
  reorderHomeWidgets,
  replaceHomarrHome,
  serializeNoteWidgetConfiguration,
  serializeWeatherWidgetConfiguration,
  type HomeTemplate,
  type HomeWidget,
  type HomeWidgetKind,
  type NoteWidgetConfiguration,
} from './home-layout';
import { DockerInventoryPanel } from './docker-panel';
import { getHomarrCatalog, type HomarrWidgetCandidate } from './docker-api';
import { shortcutIconUrl, shortcutLetter } from './shortcut-icons';
import { Sparkline } from './dashboard-ui';
import { Icon, type IconName } from './icons';
import { InfoTip } from './info-tip';
import type { SystemOverview } from './types';
import { getWeatherForecast, type WeatherForecast } from './weather-api';
import { StrandFrame } from './strand-frame';
import { listStrands, type StrandSummary } from './strands-api';

interface HomeLiveData {
  overview: SystemOverview | null;
  inventory: HostInventory | null;
  servers: ManagedServer[];
}

export interface HomePageProps extends HomeLiveData {
  csrfToken: string;
  displayName: string;
  templates: HomeTemplate[];
  activeHomeId: string;
  syncStatus: 'loading' | 'synced' | 'saving' | 'local';
  homeFocus: boolean;
  canManageDocker: boolean;
  onHomeChange: (templates: HomeTemplate[], activeHomeId: string) => void;
  onHomeFocusToggle: () => void;
  onSessionExpired: () => void;
}

const widgetIcons: Record<HomeWidgetKind, IconName> = {
  clock: 'clock',
  host: 'activity',
  servers: 'servers',
  storage: 'storage',
  weather: 'weather',
  note: 'note',
  shortcut: 'external',
  graphs: 'activity',
  docker: 'host',
  strand: 'strands',
};

function ShortcutMark({ name, url, icon, size = 22 }: { name: string; url: string; icon?: string | null; size?: number }) {
  const resolved = shortcutIconUrl({ name, url, icon: icon ?? null });
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    setFailed(false);
  }, [resolved]);
  if (resolved !== null && !failed) {
    return <img class="home-shortcut-icon" src={resolved} alt="" width={size} height={size} onError={() => setFailed(true)} />;
  }
  return <span class="home-shortcut-badge" aria-hidden="true">{shortcutLetter(name)}</span>;
}

function makeWidget(kind: HomeWidgetKind): HomeWidget {
  const suffix = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
  const defaults: Record<HomeWidgetKind, Omit<HomeWidget, 'id' | 'kind'>> = {
    clock: { size: 'compact', height: 'medium', title: 'Right now', content: '', url: '', color: '', icon: '' },
    host: { size: 'wide', height: 'medium', title: 'Host pulse', content: '', url: '', color: '', icon: '' },
    servers: { size: 'wide', height: 'medium', title: 'Servers', content: '', url: '', color: '', icon: '' },
    storage: { size: 'wide', height: 'medium', title: 'Storage', content: '', url: '', color: '', icon: '' },
    weather: { size: 'wide', height: 'medium', title: 'Weather', content: '', url: '', color: '', icon: '' },
    note: { size: 'compact', height: 'medium', title: 'Notes', content: '', url: '', color: '', icon: '' },
    shortcut: { size: 'compact', height: 'medium', title: 'Shortcut', content: '', url: '', color: '', icon: '' },
    graphs: { size: 'wide', height: 'medium', title: 'Live graphs', content: '', url: '', color: '', icon: '' },
    docker: { size: 'wide', height: 'tall', title: 'Docker', content: '', url: '', color: '', icon: '' },
    strand: { size: 'wide', height: 'tall', title: 'Strand', content: '', url: '', color: '', icon: '' },
  };
  return { id: `${kind}-${suffix}`, kind, ...defaults[kind] };
}

function WidgetControls({
  widget,
  first,
  last,
  onMove,
  onResize,
  onHeight,
  onSettings,
  onRemove,
  onDragStart,
  onDragEnd,
}: {
  widget: HomeWidget;
  first: boolean;
  last: boolean;
  onMove: (offset: -1 | 1) => void;
  onResize: () => void;
  onHeight: () => void;
  onSettings: () => void;
  onRemove: () => void;
  onDragStart: (event: DragEvent) => void;
  onDragEnd: () => void;
}) {
  return (
    <div class="home-widget__controls" aria-label={`Arrange ${widget.title}`}>
      <button class="home-widget__drag" type="button" draggable onDragStart={onDragStart} onDragEnd={onDragEnd} title="Drag to move" aria-label={`Drag ${widget.title}`}>
        <Icon name="menu" size={14} />
      </button>
      <button type="button" disabled={first} onClick={() => onMove(-1)} aria-label={`Move ${widget.title} earlier`}>
        <Icon name="chevron" size={14} class="icon--back" />
      </button>
      <button type="button" disabled={last} onClick={() => onMove(1)} aria-label={`Move ${widget.title} later`}>
        <Icon name="chevron" size={14} />
      </button>
      <button type="button" onClick={onResize} title="Cycle widget width">
        {widget.size}
      </button>
      <button type="button" onClick={onHeight} title="Cycle widget height">
        {widget.height}
      </button>
      <button type="button" onClick={onSettings} aria-label={`Open ${widget.title} settings`} title="Widget settings">
        <Icon name="settings" size={14} />
      </button>
      <button class="is-danger" type="button" onClick={onRemove} aria-label={`Remove ${widget.title}`}>
        <Icon name="trash" size={14} />
      </button>
    </div>
  );
}

function ClockWidget() {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 1_000);
    return () => window.clearInterval(timer);
  }, []);
  return (
    <div class="home-clock">
      <strong>{now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })}</strong>
      <span>{now.toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' })}</span>
      <small>{Intl.DateTimeFormat().resolvedOptions().timeZone}</small>
    </div>
  );
}

function weatherCondition(code: number): string {
  if (code === 0) return 'Clear';
  if (code <= 3) return 'Partly cloudy';
  if (code === 45 || code === 48) return 'Fog';
  if (code <= 57) return 'Drizzle';
  if (code <= 67) return 'Rain';
  if (code <= 77) return 'Snow';
  if (code <= 82) return 'Rain showers';
  if (code <= 86) return 'Snow showers';
  return 'Thunderstorms';
}

function WeatherWidget({ widget, editing, onChange, csrfToken }: {
  widget: HomeWidget;
  editing: boolean;
  onChange: (patch: Partial<HomeWidget>) => void;
  csrfToken: string;
}) {
  const configuration = useMemo(() => parseWeatherWidgetConfiguration(widget.content), [widget.content]);
  const [forecast, setForecast] = useState<WeatherForecast | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (configuration.location.length < 2) {
      setForecast(null);
      setError(null);
      setLoading(false);
      return;
    }
    const controller = new AbortController();
    setLoading(true);
    setError(null);
    void getWeatherForecast(configuration.location, configuration.unit, csrfToken, controller.signal)
      .then((next) => setForecast(next))
      .catch((reason: unknown) => {
        if (!controller.signal.aborted) {
          setError(reason instanceof Error ? reason.message : 'Weather is unavailable.');
        }
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [configuration.location, configuration.unit, csrfToken]);

  if (editing || configuration.location.length < 2) {
    return (
      <div class="home-weather-editor">
        <label><span>City or postal code</span><input value={configuration.location} maxLength={120} placeholder="Denver, Colorado" onInput={(event) => onChange({ content: serializeWeatherWidgetConfiguration({ ...configuration, location: event.currentTarget.value }) })} /></label>
        <label><span>Temperature</span><select value={configuration.unit} onChange={(event) => onChange({ content: serializeWeatherWidgetConfiguration({ ...configuration, unit: event.currentTarget.value === 'celsius' ? 'celsius' : 'fahrenheit' }) })}><option value="fahrenheit">Fahrenheit</option><option value="celsius">Celsius</option></select></label>
        <small>Type your city. Helix looks it up through Open-Meteo. Browser location access stays off.</small>
      </div>
    );
  }
  if (error !== null) {
    return <div class="home-weather-state"><Icon name="warning" /><strong>Weather unavailable</strong><span>{error}</span></div>;
  }
  if (forecast === null) {
    return <div class="home-weather-state"><Icon name="refresh" class={loading ? 'is-spinning' : undefined} /><strong>Loading weather</strong><span>{configuration.location}</span></div>;
  }
  const locationDetail = [forecast.location.name, forecast.location.adminArea, forecast.location.country]
    .filter(Boolean)
    .join(', ');
  return (
    <div class="home-weather">
      <div class="home-weather-current">
        <span>{locationDetail}</span>
        <strong>{Math.round(forecast.current.temperature)}{forecast.current.temperatureUnit}</strong>
        <small>{weatherCondition(forecast.current.weatherCode)} · feels like {Math.round(forecast.current.apparentTemperature)}{forecast.current.temperatureUnit}</small>
        <dl><div><dt>Humidity</dt><dd>{Math.round(forecast.current.relativeHumidityPercent)}%</dd></div><div><dt>Wind</dt><dd>{Math.round(forecast.current.windSpeed)} {forecast.current.windSpeedUnit}</dd></div></dl>
      </div>
      <div class="home-weather-days">
        {forecast.daily.slice(0, 5).map((day) => <div key={day.date}><span>{new Date(`${day.date}T12:00:00`).toLocaleDateString([], { weekday: 'short' })}</span><strong>{Math.round(day.temperatureMax)}°</strong><small>{Math.round(day.temperatureMin)}° · {day.precipitationProbabilityMax === null ? '—' : `${Math.round(day.precipitationProbabilityMax)}%`}</small></div>)}
      </div>
      <a href="https://open-meteo.com/" target="_blank" rel="noopener noreferrer">Open-Meteo</a>
    </div>
  );
}

function HostWidget({ overview, inventory }: Pick<HomePageProps, 'overview' | 'inventory'>) {
  const memory = overview === null
    ? null
    : calculatePercent(overview.memory.usedBytes, overview.memory.totalBytes);
  return (
    <div class="home-stat-grid">
      <div><span>CPU</span><strong>{overview?.cpu.usagePercent === null || overview === null ? '—' : formatPercent(overview.cpu.usagePercent)}</strong><small>{inventory?.cpuModel ?? `${overview?.cpu.logicalCores ?? '—'} cores`}</small></div>
      <div><span>Memory</span><strong>{memory === null ? '—' : formatPercent(memory)}</strong><small>{overview === null ? 'Waiting for host' : `${formatBytes(overview.memory.availableBytes)} available`}</small></div>
      <div><span>Uptime</span><strong>{overview === null ? '—' : formatDuration(overview.uptimeSeconds)}</strong><small>{overview?.hostname ?? 'Host not reported'}</small></div>
      <div><span>Load</span><strong>{inventory?.loadAverage[0].toFixed(2) ?? '—'}</strong><small>{inventory === null ? '1 minute average' : `${inventory.processCount.toLocaleString()} processes`}</small></div>
    </div>
  );
}

function GraphsWidget({ overview, inventory }: Pick<HomePageProps, 'overview' | 'inventory'>) {
  const [samples, setSamples] = useState<Array<{ cpu: number | null; memory: number | null; load: number | null }>>([]);
  useEffect(() => {
    const cpu = overview?.cpu.usagePercent ?? null;
    const memory = overview === null ? null : calculatePercent(overview.memory.usedBytes, overview.memory.totalBytes);
    const load = inventory?.loadAverage[0] ?? null;
    if (cpu === null && memory === null && load === null) return;
    setSamples((current) => [...current, { cpu, memory, load }].slice(-60));
  }, [overview, inventory]);
  return (
    <div class="home-graphs">
      <div><span>CPU</span><Sparkline values={samples.map((sample) => sample.cpu)} label="CPU" /></div>
      <div><span>Memory</span><Sparkline values={samples.map((sample) => sample.memory)} label="Memory" /></div>
      <div><span>Load</span><Sparkline values={samples.map((sample) => sample.load)} label="Load" /></div>
    </div>
  );
}

function ServersWidget({ servers }: Pick<HomePageProps, 'servers'>) {
  const ordered = useMemo(
    () => servers.slice().sort((left, right) => Number(right.status === 'online') - Number(left.status === 'online')).slice(0, 5),
    [servers],
  );
  return (
    <div class="home-server-list">
      {ordered.map((server) => (
        <a href="#servers" key={server.id}>
          <span class={`status-dot status-dot--${server.status === 'online' ? 'good' : 'idle'}`} />
          <strong>{server.name}</strong>
          <small>{server.status === 'online' ? `${server.playersOnline}/${server.maxPlayers} players` : 'Offline'}</small>
          <span>{server.manager === 'helix' ? 'Helix' : 'AMP'}</span>
        </a>
      ))}
      {ordered.length === 0 && <div class="home-widget__empty">No servers yet. <a href="#servers">Open Servers</a> to create the first native instance.</div>}
    </div>
  );
}

function StorageWidget({ inventory }: Pick<HomePageProps, 'inventory'>) {
  const mounts = useMemo(
    () => (inventory?.mounts ?? []).filter((mount) => mount.sizeBytes >= 1_000_000_000).slice().sort((left, right) => right.sizeBytes - left.sizeBytes).slice(0, 4),
    [inventory],
  );
  return (
    <div class="home-storage-list">
      {mounts.map((mount) => (
        <a href="#storage" key={`${mount.source}-${mount.target}`}>
          <div><strong>{mount.target}</strong><span>{formatBytes(mount.availableBytes)} free</span></div>
          <div class="progress"><span style={{ width: `${Math.min(100, mount.usePercent)}%` }} /></div>
          <small>{formatPercent(mount.usePercent)} used · {formatBytes(mount.sizeBytes)}</small>
        </a>
      ))}
      {mounts.length === 0 && <div class="home-widget__empty">No mounted storage reported.</div>}
    </div>
  );
}

function NoteWidget({ widget, editing, onChange }: {
  widget: HomeWidget;
  editing: boolean;
  onChange: (patch: Partial<HomeWidget>) => void;
}) {
  const configuration = useMemo(() => parseNoteWidgetConfiguration(widget.content), [widget.content]);
  const activePage = configuration.pages.find((page) => page.id === configuration.activePageId)
    ?? configuration.pages[0]!;
  const save = (next: NoteWidgetConfiguration): void => {
    onChange({ content: serializeNoteWidgetConfiguration(next) });
  };
  const updatePage = (patch: Partial<typeof activePage>): void => {
    save({
      ...configuration,
      pages: configuration.pages.map((page) => page.id === activePage.id ? { ...page, ...patch } : page),
    });
  };
  const addPage = (): void => {
    if (configuration.pages.length >= 8) return;
    const id = `page-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
    const page = { id, title: `Page ${configuration.pages.length + 1}`, content: '' };
    save({ ...configuration, activePageId: id, pages: [...configuration.pages, page] });
  };
  const removePage = (): void => {
    if (configuration.pages.length <= 1) return;
    const pages = configuration.pages.filter((page) => page.id !== activePage.id);
    save({ ...configuration, activePageId: pages[0]!.id, pages });
  };
  const canEditContent = editing || configuration.editableOutsideLayout;
  const otherContentCharacters = configuration.pages
    .filter((page) => page.id !== activePage.id)
    .reduce((total, page) => total + Array.from(page.content).length, 0);
  const activePageLimit = Math.max(0, 7_000 - otherContentCharacters);

  return (
    <div class="home-note-workspace">
      <div class="home-note-pages" role="tablist" aria-label={`${widget.title} pages`}>
        {configuration.pages.map((page) => <button key={page.id} class={page.id === activePage.id ? 'is-active' : ''} type="button" role="tab" aria-selected={page.id === activePage.id} onClick={() => save({ ...configuration, activePageId: page.id })}>{page.title}</button>)}
        {editing && <button type="button" disabled={configuration.pages.length >= 8} onClick={addPage} title="Add note page"><Icon name="plus" size={13} /></button>}
      </div>
      {editing && <div class="home-note-page-settings"><label><span>Page name</span><input value={activePage.title} maxLength={48} onInput={(event) => updatePage({ title: event.currentTarget.value.trimStart() })} /></label><button class="button button--danger-quiet" type="button" disabled={configuration.pages.length <= 1} onClick={removePage}><Icon name="trash" size={13} />Remove page</button></div>}
      {canEditContent
        ? <textarea class="home-note-editor" value={activePage.content} maxLength={activePageLimit} placeholder="Write a note…" onInput={(event) => updatePage({ content: event.currentTarget.value })} />
        : <p class={`home-note${activePage.content.trim().length === 0 ? ' is-empty' : ''}`}>{activePage.content.trim() || 'No note yet. Open widget settings to allow quick editing.'}</p>}
    </div>
  );
}

function StrandHomeWidget({ widget, editing, onChange, csrfToken, onSessionExpired }: {
  widget: HomeWidget;
  editing: boolean;
  onChange: (patch: Partial<HomeWidget>) => void;
  csrfToken: string;
  onSessionExpired: () => void;
}) {
  const [options, setOptions] = useState<StrandSummary[]>([]);
  useEffect(() => {
    let mounted = true;
    void listStrands(csrfToken).then((strands) => {
      if (mounted) setOptions(strands.filter((strand) => strand.enabled && strand.hasWidget));
    }).catch(() => {
      if (mounted) setOptions([]);
    });
    return () => { mounted = false; };
  }, [csrfToken]);
  const selected = options.find((strand) => strand.id === widget.url);
  if (editing) {
    return (
      <label class="home-shortcut-editor">
        <span>Installed Strand</span>
        <select value={widget.url} onChange={(event) => {
          const next = options.find((strand) => strand.id === event.currentTarget.value);
          onChange({ url: event.currentTarget.value, title: next?.name ?? widget.title });
        }}>
          <option value="">Choose a Strand with helix:ui.widget</option>
          {options.map((strand) => <option key={strand.id} value={strand.id}>{strand.name}</option>)}
        </select>
      </label>
    );
  }
  if (selected === undefined) {
    return <div class="home-widget__empty">Enable a Strand with the widget capability, then pick it in edit mode.</div>;
  }
  return (
    <StrandFrame
      strandId={selected.id}
      uiEntry={selected.uiEntry}
      csrfToken={csrfToken}
      surface="widget"
      title={selected.name}
      onSessionExpired={onSessionExpired}
    />
  );
}

function WidgetBody({ widget, editing, onChange, data, csrfToken, canManageDocker, onSessionExpired }: {
  widget: HomeWidget;
  editing: boolean;
  onChange: (patch: Partial<HomeWidget>) => void;
  data: HomeLiveData;
  csrfToken: string;
  canManageDocker: boolean;
  onSessionExpired: () => void;
}) {
  if (widget.kind === 'clock') return <ClockWidget />;
  if (widget.kind === 'host') return <HostWidget overview={data.overview} inventory={data.inventory} />;
  if (widget.kind === 'graphs') return <GraphsWidget overview={data.overview} inventory={data.inventory} />;
  if (widget.kind === 'servers') return <ServersWidget servers={data.servers} />;
  if (widget.kind === 'storage') return <StorageWidget inventory={data.inventory} />;
  if (widget.kind === 'docker') return <DockerInventoryPanel csrfToken={csrfToken} canManage={canManageDocker} compact onSessionExpired={onSessionExpired} />;
  if (widget.kind === 'weather') return <WeatherWidget widget={widget} editing={editing} onChange={onChange} csrfToken={csrfToken} />;
  if (widget.kind === 'note') return <NoteWidget widget={widget} editing={editing} onChange={onChange} />;
  if (widget.kind === 'strand') {
    return <StrandHomeWidget widget={widget} editing={editing} onChange={onChange} csrfToken={csrfToken} onSessionExpired={onSessionExpired} />;
  }

  const href = normalizeShortcutUrl(widget.url);
  if (editing) {
    return (
      <div class="home-shortcut-editor">
        <label><span>Web address</span><input value={widget.url} maxLength={2_048} placeholder="https://example.com" inputMode="url" onInput={(event) => {
          const url = event.currentTarget.value;
          onChange({ url, icon: shortcutIconUrl({ name: widget.title, url, icon: widget.icon }) ?? '' });
        }} /></label>
        {widget.url.trim().length > 0 && href.length === 0 && <small role="alert">Use a complete http:// or https:// address.</small>}
      </div>
    );
  }
  return href.length === 0
    ? <div class="home-widget__empty">Turn on edit mode to add this shortcut’s address.</div>
    : <a class="home-shortcut" href={href} target="_blank" rel="noopener noreferrer"><ShortcutMark name={widget.title} url={href} icon={widget.icon} size={35} /><span><strong>{widget.title}</strong><small>{new URL(href).hostname}</small></span></a>;
}

function WidgetSettings({ widget, onChange, onClose }: {
  widget: HomeWidget;
  onChange: (patch: Partial<HomeWidget>) => void;
  onClose: () => void;
}) {
  const note = widget.kind === 'note' ? parseNoteWidgetConfiguration(widget.content) : null;
  return (
    <div class="home-widget-settings" role="group" aria-label={`${widget.title} settings`}>
      <div class="home-widget-settings__head"><strong>Widget settings</strong><button type="button" onClick={onClose} aria-label="Close widget settings"><Icon name="close" size={14} /></button></div>
      <div class="home-widget-settings__grid">
        <label><span>Width</span><select value={widget.size} onChange={(event) => onChange({ size: event.currentTarget.value as HomeWidget['size'] })}><option value="compact">Compact</option><option value="wide">Wide</option><option value="full">Full row</option></select></label>
        <label><span>Height</span><select value={widget.height} onChange={(event) => onChange({ height: event.currentTarget.value as HomeWidget['height'] })}><option value="short">Short</option><option value="medium">Medium</option><option value="tall">Tall</option></select></label>
        <label><span>Accent</span><span class="home-color-control"><input type="color" value={widget.color || '#d7f64d'} onInput={(event) => onChange({ color: event.currentTarget.value.toLowerCase() })} /><button type="button" onClick={() => onChange({ color: '' })}>Use Home color</button></span></label>
      </div>
      {note !== null && <label class="home-widget-toggle"><input class="toggle-input" type="checkbox" checked={note.editableOutsideLayout} onChange={(event) => onChange({ content: serializeNoteWidgetConfiguration({ ...note, editableOutsideLayout: event.currentTarget.checked }) })} /><span><strong>Quick editing</strong><small>Allow this note to be edited without turning on layout mode.</small></span></label>}
    </div>
  );
}

function downloadHomeTemplate(template: HomeTemplate): void {
  const blob = new Blob([exportHomeTemplate(template)], { type: 'application/json' });
  const href = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = href;
  anchor.download = `${template.name.toLowerCase().replace(/[^a-z0-9]+/gu, '-').replace(/^-|-$/gu, '') || 'helix-home'}.helix-home.json`;
  anchor.click();
  window.setTimeout(() => URL.revokeObjectURL(href), 0);
}

function HomeTemplatePanel({ templates, activeHomeId, onHomeChange, onClose }: {
  templates: HomeTemplate[];
  activeHomeId: string;
  onHomeChange: (templates: HomeTemplate[], activeHomeId: string) => void;
  onClose: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const active = templates.find((template) => template.id === activeHomeId) ?? templates[0]!;
  const update = (patch: Partial<HomeTemplate>): void => onHomeChange(
    templates.map((template) => template.id === active.id ? { ...template, ...patch } : template), active.id,
  );
  const duplicate = (): void => {
    if (templates.length >= 8) return;
    const id = `home-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
    const copy: HomeTemplate = {
      ...active,
      id,
      name: `${active.name} copy`.slice(0, 48),
      widgets: active.widgets.map((widget, index) => ({ ...widget, id: `${widget.kind}-${Date.now().toString(36)}-${index.toString(36)}` })),
    };
    onHomeChange([...templates, copy], id);
  };
  const create = (): void => {
    if (templates.length >= 8) return;
    const id = `home-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
    onHomeChange([...templates, { id, name: `Home ${templates.length + 1}`, accent: active.accent, widgets: [] }], id);
  };
  const remove = (): void => {
    if (templates.length <= 1 || !confirmDelete) {
      setConfirmDelete(true);
      return;
    }
    const remaining = templates.filter((template) => template.id !== active.id);
    onHomeChange(remaining, remaining[0]!.id);
    setConfirmDelete(false);
  };
  const importFile = async (file: File | undefined): Promise<void> => {
    if (file === undefined || templates.length >= 8) return;
    setError(null);
    try {
      if (file.size > 70_000) throw new Error('Template files must be 70 KB or smaller.');
      const imported = importHomeTemplate(await file.text());
      onHomeChange([...templates, imported], imported.id);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Template could not be imported.');
    }
  };

  return (
    <section class="home-template-panel" aria-label="Home templates">
      <div class="home-template-panel__head"><div><strong>Homes</strong><span>Switch workspaces or share a layout as a file.</span></div><button type="button" onClick={onClose} aria-label="Close Home templates"><Icon name="close" size={15} /></button></div>
      <div class="home-template-tabs">{templates.map((template) => <button key={template.id} class={template.id === active.id ? 'is-active' : ''} type="button" onClick={() => { onHomeChange(templates, template.id); setConfirmDelete(false); }}><span style={{ background: template.accent }} /><strong>{template.name}</strong><small>{template.widgets.length} widget{template.widgets.length === 1 ? '' : 's'}</small></button>)}</div>
      <div class="home-template-editor">
        <label><span>Name</span><input value={active.name} maxLength={48} onInput={(event) => update({ name: event.currentTarget.value.trimStart() })} /></label>
        <label><span>Home accent</span><input type="color" value={active.accent} onInput={(event) => update({ accent: event.currentTarget.value.toLowerCase() })} /></label>
        <div class="home-template-actions"><button class="button button--quiet" type="button" disabled={templates.length >= 8} onClick={create}><Icon name="plus" size={14} />New blank</button><button class="button button--quiet" type="button" disabled={templates.length >= 8} onClick={duplicate}><Icon name="file" size={14} />Duplicate</button><button class="button button--quiet" type="button" onClick={() => downloadHomeTemplate(active)}><Icon name="update" size={14} />Export</button><label class={`button button--quiet${templates.length >= 8 ? ' is-disabled' : ''}`}><Icon name="backup" size={14} />Import<input type="file" accept="application/json,.json,.helix-home.json" disabled={templates.length >= 8} onChange={(event) => void importFile(event.currentTarget.files?.[0])} /></label></div>
        <div class="home-template-delete"><button class="button button--danger-quiet" type="button" disabled={templates.length <= 1} onClick={remove}><Icon name="trash" size={14} />{confirmDelete ? `Delete ${active.name}` : 'Delete Home'}</button>{confirmDelete && <button class="button button--quiet" type="button" onClick={() => setConfirmDelete(false)}>Keep it</button>}</div>
      </div>
      {error !== null && <div class="settings-form-error" role="alert"><Icon name="warning" size={14} />{error}</div>}
      <small class="home-template-safety"><Icon name="backup" size={13} />Homes, notes, colors, and widget settings are saved in Helix’s backed-up state database. Export adds a portable copy.</small>
    </section>
  );
}

export function HomePage({ overview, inventory, servers, displayName, templates, activeHomeId, syncStatus, onHomeChange, csrfToken, homeFocus, onHomeFocusToggle, canManageDocker, onSessionExpired }: HomePageProps) {
  const [editing, setEditing] = useState(false);
  const [adding, setAdding] = useState(false);
  const [templatesOpen, setTemplatesOpen] = useState(false);
  const [settingsWidgetId, setSettingsWidgetId] = useState<string | null>(null);
  const [draggedWidgetId, setDraggedWidgetId] = useState<string | null>(null);
  const [dropTargetId, setDropTargetId] = useState<string | null>(null);
  const [dropPlacement, setDropPlacement] = useState<'before' | 'after'>('before');
  const [homarrOpen, setHomarrOpen] = useState(false);
  const [homarrLoading, setHomarrLoading] = useState(false);
  const [homarrError, setHomarrError] = useState<string | null>(null);
  const [homarrNote, setHomarrNote] = useState<string | null>(null);
  const [homarrWidgets, setHomarrWidgets] = useState<HomarrWidgetCandidate[]>([]);
  const [homarrSelected, setHomarrSelected] = useState<string[]>([]);
  const activeTemplate = templates.find((template) => template.id === activeHomeId) ?? templates[0]!;
  const widgets = activeTemplate.widgets;
  const homarrHome = templates.find((template) => template.id === HOMARR_HOME_ID);
  const existingShortcutUrls = homeShortcutUrls(homarrHome?.widgets ?? []);

  const changeWidgets = (update: (current: HomeWidget[]) => HomeWidget[]): void => {
    onHomeChange(
      templates.map((template) => template.id === activeTemplate.id ? { ...template, widgets: update(widgets) } : template),
      activeTemplate.id,
    );
  };

  const updateWidget = (id: string, patch: Partial<HomeWidget>): void => {
    changeWidgets((current) => current.map((widget) => widget.id === id ? { ...widget, ...patch } : widget));
  };
  const addWidget = (kind: HomeWidgetKind): void => {
    changeWidgets((current) => [...current, makeWidget(kind)]);
    setAdding(false);
  };
  const finishDrag = (): void => {
    setDraggedWidgetId(null);
    setDropTargetId(null);
    setDropPlacement('before');
  };
  const loadHomarr = async (): Promise<void> => {
    setHomarrLoading(true);
    setHomarrError(null);
    setHomarrNote(null);
    setHomarrOpen(true);
    try {
      const catalog = await getHomarrCatalog(csrfToken);
      const already = homeShortcutUrls(templates.find((template) => template.id === HOMARR_HOME_ID)?.widgets ?? []);
      const importable = catalog.widgets.filter((widget) => !already.has(widget.url));
      setHomarrWidgets(catalog.widgets);
      setHomarrSelected(catalog.widgets.map((widget) => widget.url));
      if (catalog.availability !== 'ready') {
        setHomarrError(catalog.note);
        setHomarrNote(null);
        return;
      }
      if (catalog.widgets.length === 0) {
        setHomarrNote(catalog.note ?? 'Homarr has no http(s) shortcuts Helix can import.');
        return;
      }
      if (templates.length >= 8 && templates.every((template) => template.id !== HOMARR_HOME_ID)) {
        setHomarrNote(catalog.note);
        setHomarrError('You already have 8 Homes. Remove one, then import Homarr again.');
        setHomarrSelected([]);
        return;
      }
      if (importable.length === 0) {
        setHomarrNote('Those Homarr apps are already on the Homarr Home. Uncheck any you want to drop, then import again to refresh the layout.');
        return;
      }
      setHomarrNote(catalog.note ?? 'Helix puts these on a Homarr Home in Homarr’s layout order, with matching icons. Main stays as it is.');
    } catch (reason) {
      setHomarrWidgets([]);
      setHomarrSelected([]);
      setHomarrNote(null);
      setHomarrError(reason instanceof Error ? reason.message : 'Helix could not read Homarr.');
    } finally {
      setHomarrLoading(false);
    }
  };
  const importHomarr = (): void => {
    const chosen = homarrWidgets.filter((widget) => homarrSelected.includes(widget.url));
    const additions = newHomarrShortcuts(chosen, []);
    if (additions.length === 0) return;
    const shortcuts = additions.map((widget, index) => ({
      id: `shortcut-homarr-${Date.now().toString(36)}-${index.toString(36)}`,
      kind: 'shortcut' as const,
      size: homarrShortcutSize(widget.width),
      height: 'short' as const,
      title: widget.name.slice(0, 80),
      content: '',
      url: widget.url,
      color: '',
      icon: shortcutIconUrl({ name: widget.name, url: widget.url, icon: widget.icon }) ?? '',
    }));
    const result = replaceHomarrHome(templates, shortcuts);
    if ('error' in result) {
      setHomarrError(result.error);
      return;
    }
    onHomeChange(result.templates, result.activeHomeId);
    setHomarrOpen(false);
    setAdding(false);
  };

  return (
    <div class="page page--home" style={{ '--home-accent': activeTemplate.accent }}>
      <div class="page-head home-page-head">
        <div><span class="eyebrow">{activeTemplate.name}</span><h1>Home</h1><p>{displayName.trim().length > 0 ? `Welcome, ${displayName.trim()}. Arrange this workspace around how you run this host.` : 'Your at-a-glance workspace. Arrange it around how you run this host.'}</p></div>
        <div class="page-head-actions home-page-actions">
          {editing && <button class="button button--quiet" type="button" onClick={() => setAdding((value) => !value)}><Icon name="plus" size={15} />Add widget</button>}
          {editing && <button class="button button--quiet" type="button" disabled={homarrLoading} onClick={() => void loadHomarr()}>{homarrLoading ? 'Reading Homarr…' : 'Import from Homarr'}</button>}
          <button class={`button${homeFocus ? ' button--primary' : ' button--quiet'}`} type="button" aria-pressed={homeFocus} onClick={onHomeFocusToggle}><Icon name="expand" size={15} />{homeFocus ? 'Exit full screen' : 'Full screen'}</button>
          <button class={`button${templatesOpen ? ' button--primary' : ' button--quiet'}`} type="button" aria-pressed={templatesOpen} onClick={() => setTemplatesOpen((value) => !value)}><Icon name="home" size={15} />Homes</button>
          <button class={`button${editing ? ' button--primary' : ''}`} type="button" aria-pressed={editing} onClick={() => { setEditing((value) => !value); setAdding(false); setSettingsWidgetId(null); finishDrag(); }}><Icon name={editing ? 'check' : 'edit'} size={15} />{editing ? 'Done editing' : 'Edit layout'}</button>
        </div>
      </div>
      <div class={`home-local-note home-local-note--${syncStatus}`}><Icon name={syncStatus === 'synced' ? 'check' : syncStatus === 'local' ? 'warning' : 'refresh'} size={14} /><span>{syncStatus === 'synced' ? 'Layout synced through Helix' : syncStatus === 'saving' ? 'Saving layout…' : syncStatus === 'loading' ? 'Loading your layout…' : 'Using this browser’s saved copy'}</span><InfoTip text={syncStatus === 'local' ? 'Changes remain in this browser and retry automatically.' : 'This layout follows the owner account across browsers, with a local fallback.'} /></div>
      {templatesOpen && <HomeTemplatePanel templates={templates} activeHomeId={activeTemplate.id} onHomeChange={onHomeChange} onClose={() => setTemplatesOpen(false)} />}
      {editing && homarrOpen && (
        <section class="home-widget-catalog" aria-label="Import Homarr shortcuts">
          <div>
            <strong>Homarr shortcuts</strong>
            <span>
              {homarrLoading
                ? 'Reading Homarr apps from this host…'
                : homarrWidgets.length > 0
                  ? (homarrNote ?? 'Helix puts these on a Homarr Home in Homarr’s layout order, with matching icons. Main stays as it is.')
                  : (homarrError ?? homarrNote ?? 'Helix puts these on a Homarr Home in Homarr’s layout order, with matching icons.')}
            </span>
          </div>
          {homarrError !== null && homarrWidgets.length > 0 && <p class="home-homarr-error" role="status">{homarrError}</p>}
          {homarrWidgets.map((widget) => {
            const alreadyOnHome = existingShortcutUrls.has(widget.url);
            return (
              <label key={widget.url} class={`home-homarr-item${alreadyOnHome ? ' is-present' : ''}`}>
                <input
                  type="checkbox"
                  checked={homarrSelected.includes(widget.url)}
                  onChange={() => setHomarrSelected((current) => current.includes(widget.url) ? current.filter((item) => item !== widget.url) : [...current, widget.url])}
                />
                <ShortcutMark name={widget.name} url={widget.url} icon={widget.icon} size={28} />
                <span>
                  <strong>{widget.name}</strong>
                  <small>{alreadyOnHome ? 'Already on Homarr Home' : widget.url}</small>
                </span>
              </label>
            );
          })}
          <div class="home-homarr-actions">
            <button class="button button--quiet" type="button" onClick={() => setHomarrOpen(false)}>Cancel</button>
            <button class="button button--primary" type="button" disabled={homarrSelected.length === 0 || homarrLoading} onClick={importHomarr}>Place {homarrSelected.length} on Homarr Home</button>
          </div>
        </section>
      )}
      {editing && adding && (
        <section class="home-widget-catalog" aria-label="Add a widget">
          <div><strong>Add a widget</strong><span>Every widget can be moved and resized after it is added.</span></div>
          {(['clock', 'host', 'graphs', 'servers', 'storage', 'docker', 'weather', 'note', 'shortcut', 'strand'] as const).map((kind) => <button type="button" key={kind} onClick={() => addWidget(kind)}><Icon name={widgetIcons[kind]} /><span><strong>{kind === 'host' ? 'Host pulse' : kind === 'graphs' ? 'Live graphs' : kind === 'docker' ? 'Docker' : kind === 'strand' ? 'Strand' : kind[0]?.toUpperCase() + kind.slice(1)}</strong><small>{kind === 'shortcut' ? 'Open a website' : kind === 'note' ? 'Keep synced notes' : kind === 'weather' ? 'Five-day forecast' : kind === 'graphs' ? 'CPU, memory, and load' : kind === 'docker' ? 'All containers on this host' : kind === 'strand' ? 'An installed Strand page' : 'Live dashboard data'}</small></span></button>)}
        </section>
      )}
      {editing && <div class="home-editing-hint"><Icon name="menu" size={14} /><span>Drag a widget by its handle, or use the arrow controls. Width, height, color, and widget-specific options are under Settings.</span></div>}
      <section class={`home-grid${editing ? ' is-editing' : ''}`} aria-label="Home widgets">
        {widgets.map((widget, index) => (
          <article
            class={`home-widget home-widget--${widget.size} home-widget--height-${widget.height} home-widget--kind-${widget.kind}${draggedWidgetId === widget.id ? ' is-dragging' : ''}${dropTargetId === widget.id ? ` is-drop-${dropPlacement}` : ''}`}
            key={widget.id}
            style={widget.color.length > 0 ? { '--widget-accent': widget.color } : undefined}
            onDragOver={(event) => {
              if (!editing || draggedWidgetId === null || draggedWidgetId === widget.id) return;
              event.preventDefault();
              const bounds = event.currentTarget.getBoundingClientRect();
              const verticalLayout = bounds.width >= window.innerWidth * 0.7;
              const placement = verticalLayout
                ? (event.clientY < bounds.top + bounds.height / 2 ? 'before' : 'after')
                : (event.clientX < bounds.left + bounds.width / 2 ? 'before' : 'after');
              setDropTargetId(widget.id);
              setDropPlacement(placement);
              if (event.clientY < 72) window.scrollBy({ top: -18, behavior: 'auto' });
              else if (event.clientY > window.innerHeight - 72) window.scrollBy({ top: 18, behavior: 'auto' });
            }}
            onDragLeave={() => { if (dropTargetId === widget.id) setDropTargetId(null); }}
            onDrop={(event) => { event.preventDefault(); if (draggedWidgetId !== null) changeWidgets((current) => reorderHomeWidgets(current, draggedWidgetId, widget.id, dropPlacement)); finishDrag(); }}
          >
            <header>
              <div>{widget.kind === 'shortcut' ? <ShortcutMark name={widget.title} url={widget.url} icon={widget.icon} size={16} /> : <Icon name={widgetIcons[widget.kind]} size={16} />}{editing ? <input class="home-widget__title-input" value={widget.title} maxLength={80} aria-label={`${widget.kind} widget title`} onInput={(event) => updateWidget(widget.id, { title: event.currentTarget.value })} /> : <h2>{widget.title}</h2>}</div>
              {editing && <WidgetControls widget={widget} first={index === 0} last={index === widgets.length - 1} onMove={(offset) => changeWidgets((current) => moveHomeWidget(current, widget.id, offset))} onResize={() => updateWidget(widget.id, { size: nextHomeWidgetSize(widget.size) })} onHeight={() => updateWidget(widget.id, { height: nextHomeWidgetHeight(widget.height) })} onSettings={() => setSettingsWidgetId((current) => current === widget.id ? null : widget.id)} onDragStart={(event) => { setDraggedWidgetId(widget.id); event.dataTransfer?.setData('text/plain', widget.id); if (event.dataTransfer !== null) event.dataTransfer.effectAllowed = 'move'; }} onDragEnd={finishDrag} onRemove={() => changeWidgets((current) => current.filter((candidate) => candidate.id !== widget.id))} />}
            </header>
            {editing && settingsWidgetId === widget.id && <WidgetSettings widget={widget} onChange={(patch) => updateWidget(widget.id, patch)} onClose={() => setSettingsWidgetId(null)} />}
            <WidgetBody widget={widget} editing={editing} onChange={(patch) => updateWidget(widget.id, patch)} data={{ overview, inventory, servers }} csrfToken={csrfToken} canManageDocker={canManageDocker} onSessionExpired={onSessionExpired} />
          </article>
        ))}
        {widgets.length === 0 && <div class="home-grid-empty"><Icon name="overview" size={26} /><strong>This Home is empty</strong><span>Use Add widget to build the layout you want.</span></div>}
      </section>
    </div>
  );
}
