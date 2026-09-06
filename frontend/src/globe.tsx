import { useEffect, useState } from 'preact/hooks';
import { ApiError } from './api';
import { InlineError, PageHead } from './dashboard-ui';
import { GlobeMap } from './globe-map';
import { getGlobeSnapshot, type GlobeSnapshot } from './globe-api';
import { Icon } from './icons';
import { InfoTip } from './info-tip';

const FLOW_STORAGE_KEY = 'helix.globe.flow';
const POLL_MS = 8_000;

function readFlowPreference(): boolean {
  try {
    return globalThis.localStorage?.getItem(FLOW_STORAGE_KEY) === '1';
  } catch {
    return false;
  }
}

function saveFlowPreference(value: boolean): void {
  try {
    if (value) globalThis.localStorage?.setItem(FLOW_STORAGE_KEY, '1');
    else globalThis.localStorage?.removeItem(FLOW_STORAGE_KEY);
  } catch {
    // Flow still works in memory if browser storage is unavailable.
  }
}

function describeLink(peers: number, kind: 'player' | 'outbound'): string {
  if (kind === 'player') {
    return peers === 1 ? '1 connection on a hosted game port' : `${peers} connections on hosted game ports`;
  }
  return peers === 1 ? '1 outbound connection' : `${peers} outbound connections`;
}

export function GlobePage({
  csrfToken,
  onSessionExpired,
}: {
  csrfToken: string;
  onSessionExpired: () => void;
}) {
  const [snapshot, setSnapshot] = useState<GlobeSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [flow, setFlow] = useState(readFlowPreference);

  useEffect(() => {
    let mounted = true;
    let timer = 0;
    const controller = new AbortController();
    const load = async (): Promise<void> => {
      if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
      try {
        const next = await getGlobeSnapshot(csrfToken, controller.signal);
        if (!mounted) return;
        setSnapshot(next);
        setError(null);
      } catch (reason) {
        if (!mounted || controller.signal.aborted) return;
        if (reason instanceof ApiError && reason.status === 401) {
          onSessionExpired();
          return;
        }
        setError(reason instanceof Error ? reason.message : 'Helix could not load the globe.');
      }
    };
    void load();
    const tick = (): void => {
      if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
      void load();
    };
    timer = window.setInterval(tick, POLL_MS);
    document.addEventListener('visibilitychange', tick);
    return () => {
      mounted = false;
      controller.abort();
      window.clearInterval(timer);
      document.removeEventListener('visibilitychange', tick);
    };
  }, [csrfToken, onSessionExpired]);

  const toggleFlow = (): void => {
    setFlow((current) => {
      const next = !current;
      saveFlowPreference(next);
      return next;
    });
  };

  const origin = snapshot?.origin;
  const players = snapshot?.links.filter((link) => link.kind === 'player') ?? [];
  const outbound = snapshot?.links.filter((link) => link.kind === 'outbound') ?? [];

  return (
    <div class="page page--globe">
      <PageHead
        title="Globe"
        detail="Country-level view of this host and where established connections are going. Game-port lines include pings and join attempts, not only people who made it in."
        actions={(
          <button class={`button${flow ? ' button--primary' : ' button--quiet'}`} type="button" aria-pressed={flow} onClick={toggleFlow}>
            {flow ? 'Data motion on' : 'Solid lines'}
          </button>
        )}
      />
      <InlineError message={error} />
      <div class="globe-stage">
        <div class="globe-toolbar">
          <div class="globe-legend">
            <span><i class="is-origin" />This host</span>
            <span><i class="is-player" />Joins and pings on hosted games</span>
            <span><i class="is-outbound" />Other outbound sockets</span>
            <InfoTip text="Pins are country centroids, not streets. Helix looks up addresses on the host and never sends remote IPs to the browser. An open public game port is found by scanners even if you never shared the address; those attempts show up here the same way a real join does. Motion is optional and stays in this tab; it is not a live bytes-per-second meter." />
          </div>
          <small>{flow ? 'Dots travel faster where more sessions or queued traffic share a country.' : 'Turn on data motion if you want the lines to pulse with activity.'}</small>
        </div>
        {snapshot === null && error === null && (
          <div class="detail-loading" aria-busy="true"><Icon name="globe" size={28} /><span>Reading connections…</span></div>
        )}
        {snapshot !== null && <GlobeMap snapshot={snapshot} flow={flow} />}
        {origin !== undefined && !origin.available && (
          <div class="globe-status">
            <strong>This host is not placed yet</strong>
            <span>{origin.note}</span>
            <small>Destination countries still appear. Helix will not invent a pin at 0,0.</small>
          </div>
        )}
        {origin?.available && (
          <div class="globe-status">
            <strong>{origin.label} · {origin.countryName ?? origin.country}</strong>
            <span>{origin.note}</span>
          </div>
        )}
        {snapshot !== null && snapshot.links.length === 0 && (
          <div class="globe-empty">
            <strong>No public destinations right now</strong>
            <span>Helix only maps established TCP sockets to globally routable addresses. Loopback, LAN, CGNAT, and Tailscale overlays stay off the map.</span>
          </div>
        )}
        {snapshot !== null && snapshot.links.length > 0 && (
          <div class="globe-link-list">
            {[...players, ...outbound].map((link) => (
              <article class="globe-link-card" key={link.id}>
                <span class="globe-link-kind">{link.kind === 'player' ? 'Game port' : 'Outbound'}</span>
                <strong>{link.countryName}</strong>
                <small>{describeLink(link.peers, link.kind)}{link.servers.length > 0 ? ` · ${link.servers.join(', ')}` : ''}</small>
              </article>
            ))}
          </div>
        )}
        {snapshot?.truncated && <small>The snapshot hit the socket or country cap, so quieter destinations may be omitted.</small>}
        {snapshot !== null && <small>{snapshot.note}</small>}
      </div>
    </div>
  );
}
