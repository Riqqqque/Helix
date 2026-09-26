import { useEffect, useMemo, useState } from 'preact/hooks';
import { ApiError } from './api';
import { type ExtraPort, type ExtraPortProtocol, setNativeExtraPorts } from './control-api';
import { InlineError } from './dashboard-ui';
import { Icon } from './icons';
import './server-ports.css';

const MAX_PORTS = 16;

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : 'Helix could not save the ports.';
}

function isSessionError(error: unknown): boolean {
  return error instanceof ApiError && (error.status === 401 || error.code === 'csrf_rejected');
}

interface PortPreset {
  label: string;
  port: number;
  protocol: ExtraPortProtocol;
  hint: string;
}

const MINECRAFT_PRESETS: PortPreset[] = [
  { label: 'Simple Voice Chat', port: 24_454, protocol: 'udp', hint: 'Set port=24454 in voicechat-server.properties (plugins/voicechat/ or config/voicechat/).' },
  { label: 'BlueMap', port: 8_100, protocol: 'tcp', hint: 'BlueMap’s webserver.conf uses 8100 by default.' },
  { label: 'Dynmap', port: 8_123, protocol: 'tcp', hint: 'Dynmap’s configuration.txt uses webserver-port 8123 by default.' },
  { label: 'Geyser (Bedrock)', port: 19_132, protocol: 'udp', hint: 'Geyser listens on 19132 UDP by default.' },
];

const PROTOCOL_LABEL: Record<ExtraPortProtocol, string> = { tcp: 'TCP', udp: 'UDP', both: 'TCP + UDP' };

export function validatePortDraft(ports: ExtraPort[], reserved: number[]): string | null {
  if (ports.length > MAX_PORTS) return `A server can have at most ${MAX_PORTS} extra ports.`;
  const seen = new Set<number>();
  for (const entry of ports) {
    if (!Number.isInteger(entry.port) || entry.port < 1_024 || entry.port > 65_535) return 'Ports must be whole numbers from 1024 to 65535.';
    if (reserved.includes(entry.port)) return `Port ${entry.port} is already this server’s game or query port.`;
    if (seen.has(entry.port)) return `Port ${entry.port} is listed twice. Use TCP + UDP for one port that needs both.`;
    if (entry.label.trim().length > 40) return 'Labels must be 40 characters or fewer.';
    seen.add(entry.port);
  }
  return null;
}

function samePorts(left: ExtraPort[], right: ExtraPort[]): boolean {
  const key = (ports: ExtraPort[]) => JSON.stringify([...ports].sort((a, b) => a.port - b.port).map((entry) => [entry.port, entry.protocol, entry.label.trim()]));
  return key(left) === key(right);
}

export function ServerPortsCard({
  serverId,
  kind,
  gamePort,
  queryPort,
  joinProtocol,
  extraPorts,
  lanAddress,
  running,
  csrfToken,
  canManageServers,
  onSaved,
  onSessionExpired,
}: {
  serverId: string;
  kind: string;
  gamePort: number;
  queryPort: number | null;
  joinProtocol: string;
  extraPorts: ExtraPort[];
  lanAddress: string | null;
  running: boolean;
  csrfToken: string;
  canManageServers: boolean;
  onSaved: () => Promise<void> | void;
  onSessionExpired: () => void;
}) {
  const [draft, setDraft] = useState<ExtraPort[]>(extraPorts);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  useEffect(() => { setDraft(extraPorts); }, [extraPorts]);
  const reserved = useMemo(() => [gamePort, ...(queryPort === null ? [] : [queryPort])], [gamePort, queryPort]);
  const dirty = !samePorts(draft, extraPorts);
  const draftError = validatePortDraft(draft, reserved);
  const presets = kind === 'minecraft' ? MINECRAFT_PRESETS.filter((preset) => !draft.some((entry) => entry.port === preset.port)) : [];
  const manageTitle = canManageServers ? undefined : 'Requires games.manage permission';

  const update = (index: number, change: Partial<ExtraPort>) => setDraft((current) => current.map((entry, position) => position === index ? { ...entry, ...change } : entry));
  const add = (entry: ExtraPort) => { setNotice(null); setDraft((current) => current.length >= MAX_PORTS ? current : [...current, entry]); };
  const save = async (): Promise<void> => {
    if (draftError !== null) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const result = await setNativeExtraPorts(serverId, draft, csrfToken);
      setDraft(result.extraPorts);
      if (result.changed) {
        setNotice(result.wasRunning ? 'Ports saved. The server was recreated with them and started again.' : 'Ports saved. They open the next time the server starts.');
        await onSaved();
      }
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section class="server-ports" aria-labelledby="server-ports-title">
      <header class="server-ports__head">
        <div>
          <h2 id="server-ports-title">Ports</h2>
          <p>Open extra ports for plugins and mods that listen on their own port, such as voice chat or a web map. Each port is published on this host with the same number inside the server.</p>
        </div>
      </header>
      <ul class="server-ports__list">
        <li class="server-ports__fixed">
          <span class="server-ports__badge">Game</span>
          <strong>{gamePort}</strong>
          <span>{joinProtocol}</span>
          <small>Managed by Helix</small>
        </li>
        {queryPort !== null && (
          <li class="server-ports__fixed">
            <span class="server-ports__badge">Query</span>
            <strong>{queryPort}</strong>
            <span>Second game port</span>
            <small>Managed by Helix</small>
          </li>
        )}
        {draft.map((entry, index) => (
          <li class="server-ports__row" key={index}>
            <label>
              <span>Port</span>
              <input type="number" inputMode="numeric" min={1024} max={65535} value={entry.port} disabled={!canManageServers || busy} title={manageTitle} onInput={(event) => update(index, { port: Number(event.currentTarget.value) })} />
            </label>
            <label>
              <span>Protocol</span>
              <select value={entry.protocol} disabled={!canManageServers || busy} title={manageTitle} onChange={(event) => update(index, { protocol: event.currentTarget.value as ExtraPortProtocol })}>
                {(['udp', 'tcp', 'both'] as const).map((protocol) => <option key={protocol} value={protocol}>{PROTOCOL_LABEL[protocol]}</option>)}
              </select>
            </label>
            <label class="server-ports__label">
              <span>What uses it</span>
              <input maxLength={40} placeholder="e.g. Voice chat" value={entry.label} disabled={!canManageServers || busy} title={manageTitle} onInput={(event) => update(index, { label: event.currentTarget.value })} />
            </label>
            <div class="server-ports__meta">
              {lanAddress !== null && Number.isInteger(entry.port) && entry.port >= 1_024 && <code>{lanAddress}:{entry.port}</code>}
              <button class="icon-button" type="button" aria-label={`Remove port ${entry.port}`} disabled={!canManageServers || busy} title={manageTitle} onClick={() => { setNotice(null); setDraft((current) => current.filter((_, position) => position !== index)); }}>
                <Icon name="trash" size={15} />
              </button>
            </div>
          </li>
        ))}
      </ul>
      {draft.length === 0 && <p class="server-ports__empty">No extra ports yet.</p>}
      <div class="server-ports__add">
        <button class="button button--quiet" type="button" disabled={!canManageServers || busy || draft.length >= MAX_PORTS} title={manageTitle} onClick={() => add({ port: gamePort + 100 <= 65_535 ? gamePort + 100 : 25_600, protocol: 'udp', label: '' })}>
          <Icon name="plus" size={14} />Add port
        </button>
        {presets.map((preset) => (
          <button class="server-ports__preset" type="button" key={preset.port} title={preset.hint} disabled={!canManageServers || busy || draft.length >= MAX_PORTS} onClick={() => add({ port: preset.port, protocol: preset.protocol, label: preset.label })}>
            + {preset.label} <small>{preset.port} {PROTOCOL_LABEL[preset.protocol]}</small>
          </button>
        ))}
      </div>
      {draft.some((entry) => entry.label === 'Simple Voice Chat') && (
        <p class="server-ports__hint"><Icon name="info" size={14} />Simple Voice Chat must use the same port: set <code>port=24454</code> in <code>voicechat-server.properties</code> (Files → <code>plugins/voicechat/</code> or <code>config/voicechat/</code>).</p>
      )}
      <InlineError message={dirty ? (draftError ?? error) : error} />
      {notice !== null && <p class="server-ports__notice" role="status">{notice}</p>}
      <footer class="server-ports__foot">
        <span>{running ? 'Saving recreates the container and restarts the server — players are disconnected for about a minute.' : 'Saving recreates the stopped container; nothing starts.'} Players outside your network also need these ports forwarded on your router.</span>
        {dirty && <button class="button button--quiet" type="button" disabled={busy} onClick={() => { setDraft(extraPorts); setError(null); }}>Discard</button>}
        <button class="button button--primary" type="button" disabled={!canManageServers || !dirty || busy || draftError !== null} title={manageTitle} onClick={() => void save()}>
          {busy ? 'Saving…' : 'Save ports'}
        </button>
      </footer>
    </section>
  );
}
