import { useState } from 'preact/hooks';
import type { HostInventory } from './control-api';
import type { DashboardData } from './dashboard-model';
import { InlineError, PageHead, ProgressBar, toneForPercent } from './dashboard-ui';
import { DockerInventoryRoute } from './docker-panel-route';
import { FileManagerRoute } from './file-manager-route';
import { formatBytes, formatDuration, formatPercent } from './format';
import { Icon } from './icons';
import { InfoTip } from './info-tip';
import { dismissNotice, isDismissed } from './dismissals';
import { HostUpdatesRoute, NetworkOperationsRoute } from './infrastructure-routes';
import type { StorageAnalysisMode } from './storage-analysis-api';

export function DiskMap({
  inventory,
  onBrowse,
  onAnalyze,
}: {
  inventory: HostInventory | null;
  onBrowse: (path: string) => void;
  onAnalyze: (path: string) => void;
}) {
  const disks = inventory?.disks.filter((disk) => disk.deviceType === 'disk') ?? [];
  return (
    <section class="disk-map">
      {disks.map((disk) => {
        const mounts = inventory?.mounts.filter((mount) => disk.path !== null && mount.source.startsWith(disk.path)) ?? [];
        const primary = mounts.sort((a, b) => b.sizeBytes - a.sizeBytes)[0];
        return (
          <article class={'disk-tile' + (primary === undefined ? ' is-unmounted' : '')} key={disk.name}>
            <div class="disk-tile-icon"><Icon name="storage" size={22} /></div>
            <div class="disk-tile-copy"><span>{disk.model?.trim() || disk.name}</span><strong>{formatBytes(disk.sizeBytes)}</strong><small>{primary === undefined ? 'Not mounted' : `${primary.target} · ${formatBytes(primary.availableBytes)} free`}</small></div>
            {primary !== undefined && <div class="disk-tile-usage"><span>{formatPercent(primary.usePercent)}</span><ProgressBar value={primary.usePercent} tone={toneForPercent(primary.usePercent)} /></div>}
            {primary !== undefined && <div class="disk-tile-actions"><button class="button button--quiet" type="button" onClick={() => onBrowse(primary.target)}><Icon name="folder" size={14} />Browse files</button><button class="button button--primary" type="button" onClick={() => onAnalyze(primary.target)}><Icon name="search" size={14} />Analyze space</button></div>}
          </article>
        );
      })}
    </section>
  );
}

export function StoragePage({ data, csrfToken, onSessionExpired }: { data: DashboardData; csrfToken: string; onSessionExpired: () => void }) {
  const [browsePath, setBrowsePath] = useState('/');
  const [analysis, setAnalysis] = useState<{ path: string; mode: StorageAnalysisMode } | null>(null);
  const [introHidden, setIntroHidden] = useState(() => isDismissed('storage-space-intro'));
  const openDriveAnalysis = (path: string): void => setAnalysis({ path, mode: 'thorough' });
  return (
    <div class="page page--storage">
      <PageHead title="Storage" detail="See what is using each drive, then manage files safely." />
      <InlineError message={data.inventory.error} />
      {!introHidden && (
        <section class="storage-space-intro">
          <button class="storage-space-intro__dismiss" type="button" aria-label="Dismiss space analyzer intro" onClick={() => { dismissNotice('storage-space-intro'); setIntroHidden(true); }}>
            <Icon name="close" size={14} />
          </button>
          <div>
            <span class="eyebrow">SPACE ANALYZER</span>
            <h2>Find what’s using your drive</h2>
            <p>Choose <strong>Analyze space</strong> on any mounted drive. Helix reads filesystem metadata in the background and ranks the files and folder trees consuming the most disk space.</p>
          </div>
          <div class="storage-space-intro__facts">
            <span><Icon name="activity" size={14} />Low-impact scan</span>
            <span><Icon name="storage" size={14} />Allocated disk usage</span>
            <span><Icon name="check" size={14} />Read-only until you choose trash</span>
          </div>
        </section>
      )}
      <DiskMap inventory={data.inventory.data} onBrowse={setBrowsePath} onAnalyze={openDriveAnalysis} />
      <div class="section-title section-title--spaced">
        <div>
          <h2>Files <InfoTip text="Helix can browse the host but only changes files inside storage roots allowed by the privileged broker. Delete actions move items into a recoverable .helix-trash folder." /></h2>
          <p>Browse the whole host. Changes are limited to configured storage roots.</p>
        </div>
        <button class="button button--quiet" type="button" onClick={() => setAnalysis({ path: browsePath, mode: 'quick' })}>
          <Icon name="search" size={15} />Analyze current folder
        </button>
      </div>
      <FileManagerRoute csrfToken={csrfToken} onSessionExpired={onSessionExpired} initialPath={browsePath} analysis={analysis} onAnalysisClose={() => setAnalysis(null)} />
    </div>
  );
}

export function NetworkPage({ data, csrfToken, canManageFirewall, onSessionExpired }: { data: DashboardData; csrfToken: string; canManageFirewall: boolean; onSessionExpired: () => void }) {
  const inventory = data.inventory.data;
  return (
    <div class="page page--network">
      <PageHead title="Network" detail="Interfaces, routing, listeners, and game port allocation." />
      <InlineError message={data.inventory.error} />
      <div class="section-title section-title--plain"><div><h2>Interfaces <InfoTip text="A network interface is a physical, virtual, bridge, or tunnel connection. Its addresses show how this host can be reached on each network." /></h2><p>Physical, virtual, container, and tunnel connections</p></div></div>
      <section class="interface-grid">
        {(inventory?.interfaces ?? []).map((item) => (
          <div class="interface-panel" key={item.name}>
            <header><div><span class={`status-dot status-dot--${item.state.toLowerCase() === 'up' ? 'good' : 'idle'}`} /><strong>{item.name}</strong></div><span>{item.state}</span></header>
            <div class="interface-addresses">{item.addresses.length === 0 ? <span>No addresses</span> : item.addresses.map((address) => <code key={`${address.family}-${address.address}`}>{address.address}/{address.prefixLength}</code>)}</div>
            <dl><div><dt>Received</dt><dd>{formatBytes(item.receivedBytes)}</dd></div><div><dt>Sent</dt><dd>{formatBytes(item.transmittedBytes)}</dd></div><div><dt>MTU</dt><dd>{item.mtu}</dd></div><div><dt>Errors</dt><dd>{item.receivedErrors + item.transmittedErrors}</dd></div></dl>
            <small>{item.mac ?? 'No hardware address'}</small>
          </div>
        ))}
      </section>
      <section class="surface infrastructure-section">
        <div class="section-title"><div><h2>Routes <InfoTip text="Routes decide which interface and gateway Linux uses for each destination. The default route handles traffic that has no more specific match." /></h2><p>Kernel routing table</p></div></div>
        <div class="table-scroll"><table class="data-table"><thead><tr><th>Destination</th><th>Gateway</th><th>Interface</th><th>Metric</th></tr></thead><tbody>{(inventory?.routes ?? []).map((route, index) => <tr key={`${route.destination}-${route.gateway}-${index}`}><td><code>{route.destination}</code></td><td>{route.gateway ?? 'Direct'}</td><td>{route.interface ?? '—'}</td><td>{route.metric ?? '—'}</td></tr>)}</tbody></table></div>
      </section>
      <NetworkOperationsRoute csrfToken={csrfToken} canManageFirewall={canManageFirewall} onSessionExpired={onSessionExpired} />
    </div>
  );
}

export function HostPage({ data, csrfToken, canManageDocker, onSessionExpired }: { data: DashboardData; csrfToken: string; canManageDocker: boolean; onSessionExpired: () => void }) {
  const overview = data.overview.data;
  const inventory = data.inventory.data;
  return (
    <div class="page page--host">
      <PageHead title="Host" detail="Operating system, services, and the processes using this machine." /><InlineError message={data.overview.error ?? data.inventory.error} />
      <section class="host-facts"><div><span>Hostname</span><strong>{overview?.hostname ?? '—'}</strong></div><div><span>Operating system</span><strong>{overview?.operatingSystem ?? '—'}</strong></div><div><span>Kernel</span><strong>{overview?.kernelVersion ?? '—'}</strong></div><div><span>Architecture</span><strong>{overview?.architecture ?? '—'}</strong></div><div><span>Processor</span><strong>{inventory?.cpuModel ?? (overview === null ? '—' : `${overview.cpu.logicalCores} cores`)}</strong></div><div><span>Processes <InfoTip text="Linux processes, not Windows Task Manager’s list. Each process is a running program. Threads are workers inside those programs. Docker, Plex, and game servers add a lot of threads." /></span><strong>{inventory === null ? '—' : inventory.processCount.toLocaleString()}</strong><small>{inventory === null ? 'Linux thread groups' : `${inventory.threadCount.toLocaleString()} threads`}</small></div><div><span>Uptime</span><strong>{overview === null ? '—' : formatDuration(overview.uptimeSeconds)}</strong></div><div><span>Load average</span><strong>{inventory?.loadAverage.map((value) => value.toFixed(2)).join(' / ') ?? '—'}</strong></div></section>
      <div class="host-columns">
        <section class="surface host-table-panel"><div class="section-title"><div><h2>Services <InfoTip text="Services are background programs managed by systemd. Active means systemd currently considers the unit running; failed or inactive units may need attention depending on their purpose." /></h2><p>Systemd service state</p></div></div><div class="host-table-scroll"><table class="data-table services-table"><thead><tr><th>Unit</th><th>State</th><th>Description</th></tr></thead><tbody>{(inventory?.services ?? []).map((service) => <tr key={service.unit}><td><strong>{service.unit}</strong></td><td><span class={`state-label state-label--${service.active === 'active' ? 'good' : 'idle'}`}>{service.active}</span></td><td>{service.description}</td></tr>)}</tbody></table></div></section>
        <section class="surface host-table-panel"><div class="section-title"><div><h2>Top processes <InfoTip text="A process is a running program. This list is sampled and sorted by current CPU use so a busy or runaway workload is easier to spot." /></h2><p>Sorted by current CPU use</p></div></div><div class="host-table-scroll"><table class="data-table processes-table"><thead><tr><th>Process</th><th>CPU</th><th>Memory</th><th>Uptime</th></tr></thead><tbody>{(inventory?.processes ?? []).map((process) => <tr key={process.pid}><td><strong>{process.name}</strong><small>PID {process.pid} · {process.user}</small></td><td>{formatPercent(process.cpuPercent)}</td><td>{formatBytes(process.residentBytes)}</td><td>{formatDuration(process.uptimeSeconds)}</td></tr>)}</tbody></table></div></section>
      </div>
      <HostUpdatesRoute csrfToken={csrfToken} onSessionExpired={onSessionExpired} />
      <section class="surface host-docker">
        <div class="section-title"><div><h2>Docker containers</h2><p>Every container on this host, including ones Helix did not create.</p></div></div>
        <DockerInventoryRoute csrfToken={csrfToken} canManage={canManageDocker} compact onSessionExpired={onSessionExpired} />
      </section>
    </div>
  );
}
