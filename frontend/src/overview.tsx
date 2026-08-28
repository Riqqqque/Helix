import { useEffect, useState } from 'preact/hooks';
import type { DashboardData } from './dashboard-model';
import { InlineError, Metric, PageHead, ProgressBar, Sparkline, toneForPercent } from './dashboard-ui';
import { DockerInventoryRoute } from './docker-panel-route';
import { dismissNotice, isDismissed } from './dismissals';
import { calculatePercent, formatBytes, formatDuration, formatPercent } from './format';
import { Icon } from './icons';
import { InfoTip } from './info-tip';

export interface OverviewPageProps {
  data: DashboardData;
  themeLabel: string;
  csrfToken: string;
  canManageDocker: boolean;
  onSessionExpired: () => void;
}

export function OverviewPage({ data, themeLabel, csrfToken, canManageDocker, onSessionExpired }: OverviewPageProps) {
  const { overview, inventory, servers, integration } = data;
  const [graphs, setGraphs] = useState(() => {
    try {
      return globalThis.localStorage?.getItem('helix.overview.graphs') === '1';
    } catch {
      return false;
    }
  });
  const [samples, setSamples] = useState<Array<{ cpu: number | null; memory: number | null; swap: number | null; load: number | null }>>([]);
  useEffect(() => {
    const cpu = overview.data?.cpu.usagePercent ?? null;
    const memory = overview.data === null ? null : calculatePercent(overview.data.memory.usedBytes, overview.data.memory.totalBytes);
    const swap = overview.data === null || overview.data.swap.totalBytes === 0
      ? null
      : calculatePercent(overview.data.swap.usedBytes, overview.data.swap.totalBytes);
    const load = inventory.data?.loadAverage[0] ?? null;
    if (cpu === null && memory === null && swap === null && load === null) return;
    setSamples((current) => [...current, { cpu, memory, swap, load }].slice(-60));
  }, [overview.data, inventory.data]);
  const memoryPercent = overview.data === null
    ? 0
    : calculatePercent(overview.data.memory.usedBytes, overview.data.memory.totalBytes) ?? 0;
  const swapPercent = overview.data === null || overview.data.swap.totalBytes === 0
    ? null
    : calculatePercent(overview.data.swap.usedBytes, overview.data.swap.totalBytes);
  const criticalMounts = inventory.data?.mounts.filter((mount) => mount.usePercent >= 90) ?? [];
  const online = servers.data?.filter((server) => server.status === 'online').length ?? 0;
  const helixContainers = integration.data === null
    ? []
    : [integration.data.resources.containers.dashboard, integration.data.resources.containers.gateway].filter((value) => value !== null);
  const helixCpuValues = helixContainers.flatMap((value) => value.cpuPercent === null ? [] : [value.cpuPercent]);
  const helixMemoryValues = helixContainers.flatMap((value) => value.memoryUsedBytes === null ? [] : [value.memoryUsedBytes]);
  if (integration.data?.resources.broker.rssBytes !== null && integration.data?.resources.broker.rssBytes !== undefined) {
    helixMemoryValues.push(integration.data.resources.broker.rssBytes);
  }
  const helixCpu = helixCpuValues.length === 0 ? null : helixCpuValues.reduce((sum, value) => sum + value, 0);
  const helixMemory = helixMemoryValues.length === 0 ? null : helixMemoryValues.reduce((sum, value) => sum + value, 0);
  const helixErrors = integration.data?.errors ?? [];
  const [dismissTick, setDismissTick] = useState(0);
  void dismissTick;
  const visibleMounts = criticalMounts.filter((mount) => !isDismissed(`capacity:${mount.target}`));
  const toggleGraphs = (): void => {
    setGraphs((current) => {
      const next = !current;
      try {
        if (next) globalThis.localStorage?.setItem('helix.overview.graphs', '1');
        else globalThis.localStorage?.removeItem('helix.overview.graphs');
      } catch {
        // Graph preference is optional.
      }
      return next;
    });
  };
  return (
    <div class="page page--overview">
      <PageHead
        title="Overview"
        detail={overview.data?.hostname
          ? `${overview.data.hostname} — host, storage, and game workloads in one place.`
          : 'The host, storage, and game workloads in one place.'}
        actions={<button class={`button${graphs ? ' button--primary' : ' button--quiet'}`} type="button" aria-pressed={graphs} onClick={toggleGraphs}><Icon name="activity" size={15} />{graphs ? 'Hide graphs' : 'Show graphs'}</button>}
      />
      <InlineError message={overview.error ?? inventory.error ?? servers.error ?? integration.error} />
      {visibleMounts.map((mount) => (
        <div class="capacity-alert" key={mount.target}>
          <Icon name="warning" />
          <a href="#storage">
            <strong>{mount.target} is {formatPercent(mount.usePercent)} full</strong>
            <span>{formatBytes(mount.availableBytes)} remains on {mount.source}. Open Storage to make room.</span>
          </a>
          <a class="capacity-alert__go" href="#storage" aria-label="Open Storage"><Icon name="chevron" /></a>
          <button
            class="capacity-alert__dismiss"
            type="button"
            aria-label={`Dismiss ${mount.target} warning`}
            onClick={() => {
              dismissNotice(`capacity:${mount.target}`);
              setDismissTick((value) => value + 1);
            }}
          >
            <Icon name="close" size={14} />
          </button>
        </div>
      ))}
      <section class="host-facts overview-identity">
        <div><span>Processor</span><strong>{inventory.data?.cpuModel ?? (overview.data === null ? '—' : `${overview.data.cpu.logicalCores} cores`)}</strong></div>
        <div><span>Processes</span><strong>{inventory.data === null ? '—' : inventory.data.processCount.toLocaleString()}</strong></div>
        <div><span>Virtual memory</span><strong>{overview.data === null || overview.data.swap.totalBytes === 0 ? 'No swap' : `${formatBytes(overview.data.swap.usedBytes)} / ${formatBytes(overview.data.swap.totalBytes)}`}</strong></div>
        <div><span>Operating system</span><strong>{overview.data?.operatingSystem ?? '—'}</strong></div>
        <div><span>Helix</span><strong>{overview.data?.helixVersion ?? '—'}</strong><small>{themeLabel} theme</small></div>
        <div><span>Kernel</span><strong>{overview.data?.kernelVersion ?? '—'}</strong></div>
      </section>
      {graphs && (
        <section class="overview-graphs" aria-label="Live host graphs">
          <div><span>CPU</span><Sparkline values={samples.map((sample) => sample.cpu)} label="CPU percent" /><strong>{overview.data?.cpu.usagePercent === null || overview.data === null ? '—' : formatPercent(overview.data.cpu.usagePercent)}</strong></div>
          <div><span>Memory</span><Sparkline values={samples.map((sample) => sample.memory)} label="Memory percent" /><strong>{overview.data === null ? '—' : formatPercent(memoryPercent)}</strong></div>
          <div><span>Swap</span><Sparkline values={samples.map((sample) => sample.swap)} label="Swap percent" /><strong>{swapPercent === null ? '—' : formatPercent(swapPercent)}</strong></div>
          <div><span>Load</span><Sparkline values={samples.map((sample) => sample.load)} label="One-minute load" /><strong>{inventory.data?.loadAverage[0].toFixed(2) ?? '—'}</strong></div>
        </section>
      )}
      <section class="metrics-strip" aria-label="Host metrics">
        <Metric icon="cpu" label="Processor" value={overview.data?.cpu.usagePercent === null || overview.data === null ? '—' : formatPercent(overview.data.cpu.usagePercent)} detail={`${overview.data?.cpu.logicalCores ?? '—'} logical cores`} percent={overview.data?.cpu.usagePercent ?? undefined} help="The share of this host’s CPU currently busy across all logical cores." />
        <Metric icon="memory" label="Memory" value={overview.data === null ? '—' : `${formatBytes(overview.data.memory.usedBytes)} / ${formatBytes(overview.data.memory.totalBytes)}`} detail={overview.data === null ? 'Loading' : `${formatBytes(overview.data.memory.availableBytes)} available`} percent={overview.data === null ? undefined : memoryPercent} help="RAM currently in use on the host. This includes the operating system, Helix, and every workload." />
        <Metric icon="servers" label="Game servers" value={servers.data === null ? '—' : `${online} online`} detail={servers.data === null ? 'Loading' : `${servers.data.filter((server) => server.manager === 'helix').length} Helix · ${servers.data.filter((server) => server.manager === 'amp_import').length} imported`} help="Helix-managed servers plus read-only compatibility inventory imported from AMP." />
        <Metric icon="activity" label="Uptime" value={overview.data === null ? '—' : formatDuration(overview.data.uptimeSeconds)} detail={inventory.data === null ? 'Loading' : `Load ${inventory.data.loadAverage.map((value) => value.toFixed(2)).join(' · ')}`} help="Time since the Linux host last booted. Load shows runnable work over 1, 5, and 15 minutes." />
      </section>
      <div class="overview-grid">
        <section class="surface overview-helix">
          <div class="section-title"><div><h2>Helix footprint <InfoTip text="These figures cover the Helix dashboard and gateway containers plus the privileged broker’s RAM. Game servers are deliberately excluded." /></h2><p>Dashboard runtime only · game servers excluded</p></div><span class={`state-label state-label--${integration.data?.availability === 'ready' && helixErrors.length === 0 ? 'good' : integration.data === null ? 'idle' : 'warning'}`}>{integration.data === null ? 'Loading' : helixErrors.length === 0 && integration.data.availability === 'ready' ? 'Healthy' : `${helixErrors.length} issue${helixErrors.length === 1 ? '' : 's'}`}</span></div>
          <div class="helix-footprint-stats">
            <div><span>Container CPU</span><strong>{helixCpu === null ? '—' : formatPercent(helixCpu)}</strong><small>Dashboard + gateway</small></div>
            <div><span>Runtime memory</span><strong>{helixMemory === null ? '—' : formatBytes(helixMemory)}</strong><small>Containers + broker RSS</small></div>
            <div><span>Runtime errors</span><strong>{integration.data === null ? '—' : helixErrors.length}</strong><small>{helixErrors.length === 0 ? 'No integration errors reported' : helixErrors[0]?.message}</small></div>
          </div>
        </section>
        <section class="surface overview-storage">
          <div class="section-title"><div><h2>Storage</h2><p>Mounted capacity across this host</p></div><a href="#storage">Open files <Icon name="chevron" size={14} /></a></div>
          <div class="mount-summary-list">
            {(inventory.data?.mounts ?? []).filter((mount) => mount.sizeBytes >= 10_000_000_000).slice(0, 5).map((mount) => (
              <div class="mount-summary" key={mount.target}><div class="mount-summary-top"><strong>{mount.target}</strong><span>{formatBytes(mount.usedBytes)} of {formatBytes(mount.sizeBytes)}</span></div><ProgressBar value={mount.usePercent} tone={toneForPercent(mount.usePercent)} /><small>{mount.source} · {formatBytes(mount.availableBytes)} free</small></div>
            ))}
          </div>
        </section>
        <section class="surface overview-servers">
          <div class="section-title"><div><h2>Servers</h2><p>Helix and imported workloads</p></div><a href="#servers">Manage <Icon name="chevron" size={14} /></a></div>
          <div class="compact-server-list">
            {(servers.data ?? []).slice().sort((a, b) => Number(b.status === 'online') - Number(a.status === 'online')).slice(0, 6).map((server) => (
              <div key={server.id}><span class={`status-dot status-dot--${server.status === 'online' ? 'good' : 'idle'}`} /><strong>{server.name}</strong><span>{server.status === 'online' ? `${server.playersOnline}/${server.maxPlayers} players` : 'Offline'}</span><small>{server.software} {server.version}</small></div>
            ))}
          </div>
        </section>
        <section class="surface overview-services">
          <div class="section-title"><div><h2>Services</h2><p>Systemd units Helix can see</p></div><a href="#host">Inspect <Icon name="chevron" size={14} /></a></div>
          <div class="service-pills">{(inventory.data?.services ?? []).slice(0, 8).map((service) => <span key={service.unit}><i class={`status-dot status-dot--${service.active === 'active' ? 'good' : 'idle'}`} />{service.unit.replace('.service', '')}</span>)}</div>
        </section>
        <section class="surface overview-network">
          <div class="section-title"><div><h2>Network</h2><p>Physical and virtual interfaces</p></div><a href="#network">Details <Icon name="chevron" size={14} /></a></div>
          <div class="interface-summary">{(inventory.data?.interfaces ?? []).filter((item) => item.name !== 'lo').slice(0, 4).map((item) => <div key={item.name}><span class={`status-dot status-dot--${item.state.toLowerCase() === 'up' ? 'good' : 'idle'}`} /><strong>{item.name}</strong><span>{item.addresses.find((address) => address.family === 'inet')?.address ?? 'No IPv4 address'}</span></div>)}</div>
        </section>
        <section class="surface overview-docker">
          <div class="section-title"><div><h2>Containers</h2><p>Every Docker container on this host</p></div><a href="#hooks">Open Portainer <Icon name="chevron" size={14} /></a></div>
          <DockerInventoryRoute csrfToken={csrfToken} canManage={canManageDocker} compact onSessionExpired={onSessionExpired} />
        </section>
      </div>
    </div>
  );
}
