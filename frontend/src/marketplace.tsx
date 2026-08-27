import { useCallback, useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { ApiError } from './api';
import { InlineError, ProgressBar } from './dashboard-ui';
import { Dialog } from './modal';
import { Icon } from './icons';
import {
  getMarketplaceInstallJob,
  getMarketplaceProject,
  installMarketplaceProject,
  marketplaceProfileForSoftware,
  marketplaceResponseMatchesServer,
  searchMarketplace,
  type MarketplaceInstallJob,
  type MarketplaceProjectDetail,
  type MarketplaceSearchHit,
  type MarketplaceSearchPage,
  type MarketplaceVersion,
} from './marketplace-api';
import type { MarketplaceRouteProps } from './marketplace-route';
import './marketplace.css';

const PAGE_SIZE = 20;
const SEARCH_DEBOUNCE_MS = 350;
const JOB_POLL_MS = 900;
const BODY_PREVIEW_CHARS = 12_000;

const compactNumber = new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 });
const wholeNumber = new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 });
const publishedDate = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' });

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : 'Helix could not complete that marketplace request.';
}

function isSessionError(error: unknown): boolean {
  return error instanceof ApiError && (error.status === 401 || error.code === 'csrf_rejected');
}

function formatPublished(value: string | null): string {
  if (value === null) return 'Publish date unavailable';
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? 'Publish date unavailable' : publishedDate.format(date);
}

export function defaultMarketplaceVersion(versions: readonly MarketplaceVersion[]): MarketplaceVersion | null {
  return versions.find((version) => version.versionType === 'release' && version.hasPrimaryFile) ?? null;
}

export function marketplaceInstallRuntimeCopy(status: MarketplaceRouteProps['server']['status']): {
  backup: string;
  validation: string;
} {
  if (status === 'stopped') {
    return {
      backup: 'A consistent backup is created before the verified files are staged. The server stays stopped.',
      validation: 'First startup is not automatically health-validated or rolled back. The safety backup remains available if startup fails.',
    };
  }
  return {
    backup: 'A consistent backup is created before Helix stops the running server and stages the verified files.',
    validation: 'Helix restarts the server, waits for its startup health check, and rolls the content back if that validation fails.',
  };
}

function compatibleVersion(version: MarketplaceVersion, detail: MarketplaceProjectDetail): boolean {
  return version.hasPrimaryFile
    && version.gameVersions.includes(detail.compatibility.minecraftVersion)
    && version.loaders.some((loader) => detail.compatibility.acceptedLoaders.includes(loader));
}

function ResultLettermark({ title, kind, iconUrl = null }: { title: string; kind: 'plugin' | 'mod'; iconUrl?: string | null }) {
  const [imageFailed, setImageFailed] = useState(false);
  useEffect(() => setImageFailed(false), [iconUrl]);
  const letters = title.trim().split(/\s+/u).slice(0, 2).map((word) => word[0]?.toUpperCase() ?? '').join('') || 'M';
  return <span class={`marketplace-lettermark marketplace-lettermark--${kind}`} aria-hidden="true">{iconUrl !== null && !imageFailed ? <img src={iconUrl} alt="" loading="lazy" decoding="async" onError={() => setImageFailed(true)} /> : letters}</span>;
}

function CompatibilityBar({ page }: { page: MarketplaceSearchPage }) {
  const compatibility = page.compatibility;
  return (
    <div class="marketplace-compatibility" aria-label="Search compatibility">
      <div><Icon name="check" size={15} /><span>Locked to</span><strong>{compatibility.serverSoftware} {compatibility.minecraftVersion}</strong></div>
      <span class="marketplace-kind">{compatibility.contentKind === 'plugin' ? 'Server plugins' : 'Server mods'}</span>
      <span>{compatibility.acceptedLoaders.join(' · ')}</span>
      <span><code>{compatibility.installDirectory}/</code></span>
    </div>
  );
}

function SearchCard({ hit, kind, onOpen }: { hit: MarketplaceSearchHit; kind: 'plugin' | 'mod'; onOpen: () => void }) {
  return (
    <article class="marketplace-card">
      <button class="marketplace-card__main" type="button" onClick={onOpen} aria-label={`View ${hit.title}`}>
        <ResultLettermark title={hit.title} kind={kind} iconUrl={hit.iconUrl} />
        <span class="marketplace-card__copy">
          <span class="marketplace-card__title"><strong>{hit.title}</strong>{hit.latestVersion !== null && <small>v{hit.latestVersion}</small>}</span>
          <span class="marketplace-card__description">{hit.description ?? 'No project description was provided.'}</span>
          <span class="marketplace-card__meta">
            <span>{hit.author ?? 'Unknown author'}</span>
            <span>{compactNumber.format(hit.downloads)} downloads</span>
            <span>{compactNumber.format(hit.follows)} followers</span>
          </span>
        </span>
        <Icon name="chevron" size={18} />
      </button>
    </article>
  );
}

function SearchResults({
  page,
  loading,
  error,
  onOpen,
  onRetry,
}: {
  page: MarketplaceSearchPage | null;
  loading: boolean;
  error: string | null;
  onOpen: (hit: MarketplaceSearchHit) => void;
  onRetry: () => void;
}) {
  if (loading && page === null) {
    return <div class="marketplace-state" role="status"><Icon name="refresh" class="is-spinning" /><strong>Finding compatible projects</strong><span>Modrinth results are being filtered for this exact server.</span></div>;
  }
  if (error !== null) {
    return <div class="marketplace-state marketplace-state--error" role="alert"><Icon name="warning" /><strong>Search unavailable</strong><span>{error}</span><button class="button button--primary" type="button" onClick={onRetry}>Try again</button></div>;
  }
  if (page === null) return null;
  if (page.hits.length === 0) {
    return <div class="marketplace-state"><Icon name="search" /><strong>No compatible projects found</strong><span>Try a broader name. Helix will not show content that does not match this server.</span></div>;
  }
  return <div class={`marketplace-results${loading ? ' is-refreshing' : ''}`} aria-busy={loading}>{page.hits.map((hit) => <SearchCard key={hit.projectId} hit={hit} kind={page.compatibility.contentKind} onOpen={() => onOpen(hit)} />)}</div>;
}

function VersionFacts({ version }: { version: MarketplaceVersion }) {
  return (
    <div class="marketplace-version-facts">
      <div><span>Channel</span><strong class={`marketplace-channel marketplace-channel--${version.versionType}`}>{version.versionType}</strong></div>
      <div><span>Published</span><strong>{formatPublished(version.datePublished)}</strong></div>
      <div><span>Downloads</span><strong>{wholeNumber.format(version.downloads)}</strong></div>
      <div><span>Loaders</span><strong>{version.loaders.join(', ')}</strong></div>
      <div><span>Game versions</span><strong>{version.gameVersions.join(', ')}</strong></div>
    </div>
  );
}

function InstallationResult({ job, onClose }: { job: MarketplaceInstallJob; onClose: () => void }) {
  if (job.status === 'failed') {
    return (
      <div class="marketplace-job-result marketplace-job-result--failed" role="alert">
        <span class="job-icon job-icon--failed"><Icon name="warning" size={25} /></span>
        <div><strong>Installation stopped safely</strong><p>{job.error}</p><small>Helix reports the exact broker failure above. No unverified download is kept.</small></div>
        <button class="button button--primary" type="button" onClick={onClose}>Close</button>
      </div>
    );
  }
  const result = job.result;
  if (result === null) return null;
  return (
    <div class="marketplace-job-result marketplace-job-result--complete" role="status">
      <span class="job-icon job-icon--complete"><Icon name="check" size={25} /></span>
      <div>
        <strong>{result.projectTitle} {result.versionNumber} installed</strong>
        <p>{result.runtimeValidationPerformed ? 'The running server restarted and passed its startup health check.' : 'The files are staged, but the stopped server’s first startup has not been health-validated.'}</p>
      </div>
      <dl>
        <div><dt>Backup ID</dt><dd><code>{result.backupId}</code></dd></div>
        <div><dt>Required dependencies</dt><dd>{result.dependencyCount}</dd></div>
        <div><dt>Activation</dt><dd>{result.runtimeValidationPerformed ? 'Restarted + verified' : 'First start not verified'}</dd></div>
        <div><dt>Rollback protection</dt><dd>{result.rollbackOnFailedStartup ? 'Automatic on failed startup' : 'Safety backup available'}</dd></div>
      </dl>
      {result.optionalDependenciesNotInstalled.length > 0 && <div class="marketplace-optional"><Icon name="info" size={15} /><span><strong>Optional dependencies were not installed</strong><small>{result.optionalDependenciesNotInstalled.join(', ')}</small></span></div>}
      <button class="button button--primary" type="button" onClick={onClose}>Done</button>
    </div>
  );
}

function InstallDialog({
  server,
  detail,
  version,
  csrfToken,
  onSessionExpired,
  onInstalled,
  onClose,
}: MarketplaceRouteProps & { detail: MarketplaceProjectDetail; version: MarketplaceVersion; onClose: () => void }) {
  const [confirmed, setConfirmed] = useState(false);
  const [dispatching, setDispatching] = useState(false);
  const [jobId, setJobId] = useState<string | null>(null);
  const [job, setJob] = useState<MarketplaceInstallJob | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pollRevision, setPollRevision] = useState(0);
  const installedRefresh = useRef(false);
  const active = dispatching || (error === null && (job?.status === 'queued' || job?.status === 'running'));
  const runtimeCopy = marketplaceInstallRuntimeCopy(server.status);

  useEffect(() => {
    if (jobId === null) return;
    let mounted = true;
    let timer: number | null = null;
    const controller = new AbortController();
    const poll = async (): Promise<void> => {
      try {
        const next = await getMarketplaceInstallJob(jobId, csrfToken, controller.signal);
        if (!mounted) return;
        if (next.result !== null && (next.result.instanceId !== server.id || next.result.projectId !== detail.project.id || next.result.versionId !== version.id)) {
          setError('Helix rejected an installation result that did not match this server and version.');
          return;
        }
        setJob(next);
        setError(null);
        if (next.status === 'complete') {
          if (!installedRefresh.current) {
            installedRefresh.current = true;
            void onInstalled().catch(() => undefined);
          }
          return;
        }
        if (next.status === 'failed') return;
        timer = window.setTimeout(() => void poll(), JOB_POLL_MS);
      } catch (requestError) {
        if (!mounted || controller.signal.aborted) return;
        if (isSessionError(requestError)) onSessionExpired();
        else setError(describeError(requestError));
      }
    };
    void poll();
    return () => {
      mounted = false;
      controller.abort();
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [csrfToken, detail.project.id, jobId, onInstalled, onSessionExpired, pollRevision, server.id, version.id]);

  const install = async (): Promise<void> => {
    setDispatching(true);
    setError(null);
    try {
      const dispatch = await installMarketplaceProject(server.id, detail.project.id, version.id, csrfToken);
      setJobId(dispatch.jobId);
      setJob({
        id: dispatch.jobId,
        kind: 'server_marketplace_install',
        status: 'queued',
        stage: dispatch.reused ? 'Joining existing installation' : 'Queued',
        progressPercent: 0,
        createdAtUnixMs: Date.now(),
        updatedAtUnixMs: Date.now(),
        result: null,
        error: null,
      });
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(describeError(requestError));
    } finally {
      setDispatching(false);
    }
  };

  const closeSafely = (): void => { if (!active) onClose(); };
  return (
    <Dialog title={job === null ? `Install ${detail.project.title}?` : job.status === 'complete' ? 'Installation complete' : job.status === 'failed' ? 'Installation stopped' : 'Installing content'} onClose={closeSafely} wide>
      {job === null ? <>
        <div class="marketplace-confirm-head"><ResultLettermark title={detail.project.title} kind={detail.compatibility.contentKind} iconUrl={detail.project.iconUrl} /><div><strong>{detail.project.title}</strong><span>{version.versionNumber} · {version.versionType} · {server.software} {server.minecraftVersion}</span></div></div>
        <ul class="marketplace-safety-list">
          <li><Icon name="check" size={15} /><span><strong>Required dependencies are resolved automatically.</strong> Optional dependencies stay uninstalled and will be listed afterward.</span></li>
          <li><Icon name="backup" size={15} /><span><strong>Files are changed only after a safety backup.</strong> {runtimeCopy.backup}</span></li>
          <li><Icon name="check" size={15} /><span><strong>Every downloaded file is SHA-512 verified.</strong> Helix only keeps compatible server-side content from Modrinth.</span></li>
          <li><Icon name="restart" size={15} /><span><strong>Runtime validation follows the server’s current state.</strong> {runtimeCopy.validation}</span></li>
        </ul>
        <label class="check-row marketplace-confirm-check"><input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.currentTarget.checked)} /><span><strong>Install this exact version</strong><small>I understand the server may briefly stop or restart.</small></span></label>
        <InlineError message={error} />
        <div class="dialog-actions"><button class="button button--quiet" type="button" onClick={onClose}>Cancel</button><button class="button button--primary" type="button" disabled={!confirmed || dispatching} onClick={() => void install()}>{dispatching ? 'Queuing…' : `Install ${detail.compatibility.contentKind}`}</button></div>
      </> : job.status === 'complete' || job.status === 'failed' ? <InstallationResult job={job} onClose={onClose} /> : <>
        <div class="job-progress marketplace-job-progress" role="status" aria-live="polite">
          <div class="job-icon job-icon--running"><Icon name="update" size={25} /></div>
          <strong>{job.stage}</strong>
          <span>Helix is verifying, backing up, and applying the selected content. This operation continues on the server.</span>
          <ProgressBar value={Math.max(job.progressPercent, 4)} />
          <small>{job.progressPercent}%</small>
        </div>
        <InlineError message={error} />
        {error !== null && <div class="dialog-actions"><button class="button button--primary" type="button" onClick={() => setPollRevision((value) => value + 1)}>Resume status check</button></div>}
      </>}
    </Dialog>
  );
}

function ProjectView({
  hit,
  detail,
  loading,
  error,
  canManageServers,
  selectedVersionId,
  bodyExpanded,
  onVersionChange,
  onBodyToggle,
  onBack,
  onRetry,
  onInstall,
}: {
  hit: MarketplaceSearchHit;
  detail: MarketplaceProjectDetail | null;
  loading: boolean;
  error: string | null;
  canManageServers: boolean;
  selectedVersionId: string;
  bodyExpanded: boolean;
  onVersionChange: (value: string) => void;
  onBodyToggle: () => void;
  onBack: () => void;
  onRetry: () => void;
  onInstall: () => void;
}) {
  if (loading) return <div class="marketplace-project-state" role="status"><button class="back-link" type="button" onClick={onBack}><Icon name="back" size={15} />Marketplace</button><div class="marketplace-state"><Icon name="refresh" class="is-spinning" /><strong>Loading project details</strong><span>Checking exact versions and server compatibility.</span></div></div>;
  if (error !== null || detail === null) return <div class="marketplace-project-state"><button class="back-link" type="button" onClick={onBack}><Icon name="back" size={15} />Marketplace</button><div class="marketplace-state marketplace-state--error" role="alert"><Icon name="warning" /><strong>Project unavailable</strong><span>{error ?? 'The project did not return a usable response.'}</span><button class="button button--primary" type="button" onClick={onRetry}>Try again</button></div></div>;
  const selectedVersion = detail.versions.find((version) => version.id === selectedVersionId) ?? null;
  const installable = selectedVersion !== null && compatibleVersion(selectedVersion, detail);
  const body = detail.project.body ?? detail.project.description ?? 'This project did not provide a longer description.';
  const shownBody = bodyExpanded || body.length <= BODY_PREVIEW_CHARS ? body : `${body.slice(0, BODY_PREVIEW_CHARS)}\n\n…`;
  return (
    <div class="marketplace-project">
      <button class="back-link" type="button" onClick={onBack}><Icon name="back" size={15} />Marketplace</button>
      <header class="marketplace-project-head">
        <ResultLettermark title={detail.project.title} kind={detail.compatibility.contentKind} iconUrl={detail.project.iconUrl ?? hit.iconUrl} />
        <div><span class="eyebrow">MODRINTH · {detail.compatibility.contentKind.toUpperCase()}</span><h2>{detail.project.title}</h2><p>{detail.project.description ?? 'No short description provided.'}</p><span class="marketplace-card__meta"><span>{hit.author ?? 'Unknown author'}</span><span>{compactNumber.format(detail.project.downloads)} downloads</span><span>{compactNumber.format(detail.project.followers)} followers</span></span></div>
        <a class="button button--quiet" href={detail.project.webUrl} target="_blank" rel="noreferrer">Open on Modrinth <Icon name="external" size={14} /></a>
      </header>
      <div class="marketplace-project-grid">
        <section class="marketplace-project-body"><h3>About</h3><pre>{shownBody}</pre>{body.length > BODY_PREVIEW_CHARS && <button class="button button--quiet" type="button" onClick={onBodyToggle}>{bodyExpanded ? 'Show less' : 'Read full description'}</button>}</section>
        <aside class="marketplace-install-card">
          <div><span class="eyebrow">EXACT COMPATIBILITY</span><h3>Choose a version</h3><p>Only versions returned for {detail.compatibility.serverSoftware} {detail.compatibility.minecraftVersion} are available.</p></div>
          <label class="field"><span>Version</span><select value={selectedVersionId} onChange={(event) => onVersionChange(event.currentTarget.value)}><option value="">Select a compatible version</option>{detail.versions.map((version) => <option key={version.id} value={version.id} disabled={!version.hasPrimaryFile}>{version.versionNumber} · {version.versionType}{version.hasPrimaryFile ? '' : ' · no primary file'}</option>)}</select></label>
          {selectedVersion !== null && <VersionFacts version={selectedVersion} />}
          {detail.versionResultsTruncated && <div class="marketplace-note"><Icon name="info" size={14} />Showing the newest 100 compatible versions.</div>}
          {!detail.versions.some((version) => version.versionType === 'release' && version.hasPrimaryFile) && <div class="marketplace-note marketplace-note--warning"><Icon name="warning" size={14} />No release is available. Select a beta or alpha explicitly if you accept that channel.</div>}
          {!canManageServers && <div class="marketplace-note"><Icon name="info" size={14} />Your account can browse compatible content but cannot install it.</div>}
          <button class="button button--primary marketplace-install-button" type="button" disabled={!canManageServers || !installable} onClick={onInstall}><Icon name="plus" size={15} />Review installation</button>
          <small>Installs to <code>{detail.compatibility.installDirectory}/</code>. Optional dependencies are never added silently.</small>
        </aside>
      </div>
      <footer class="marketplace-attribution">Project information and downloads are provided by <a href="https://modrinth.com" target="_blank" rel="noreferrer">Modrinth <Icon name="external" size={12} /></a>. Helix applies its own compatibility and installation safety checks.</footer>
    </div>
  );
}

export function MarketplacePanel({ server, csrfToken, canManageServers, onSessionExpired, onInstalled }: MarketplaceRouteProps) {
  const profile = marketplaceProfileForSoftware(server.software);
  const serverId = server.id;
  const serverSoftware = server.software;
  const serverMinecraftVersion = server.minecraftVersion;
  const [draftQuery, setDraftQuery] = useState('');
  const [query, setQuery] = useState('');
  const [offset, setOffset] = useState(0);
  const [searchRevision, setSearchRevision] = useState(0);
  const [page, setPage] = useState<MarketplaceSearchPage | null>(null);
  const [searching, setSearching] = useState(true);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [selectedHit, setSelectedHit] = useState<MarketplaceSearchHit | null>(null);
  const [detail, setDetail] = useState<MarketplaceProjectDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [detailRevision, setDetailRevision] = useState(0);
  const [selectedVersionId, setSelectedVersionId] = useState('');
  const [bodyExpanded, setBodyExpanded] = useState(false);
  const [installOpen, setInstallOpen] = useState(false);
  const searchTop = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      setQuery(draftQuery.trim());
      setOffset(0);
    }, SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [draftQuery]);

  useEffect(() => {
    if (profile === null) return;
    const controller = new AbortController();
    setSearching(true);
    setSearchError(null);
    void searchMarketplace(server.id, query, offset, PAGE_SIZE, csrfToken, controller.signal).then((next) => {
      if (!marketplaceResponseMatchesServer(next.instanceId, next.compatibility, server)) throw new ApiError('Marketplace compatibility changed while this page was loading. Refresh the server before installing content.');
      setPage(next);
    }).catch((requestError: unknown) => {
      if (controller.signal.aborted) return;
      if (isSessionError(requestError)) onSessionExpired();
      else {
        setSearchError(describeError(requestError));
        setPage(null);
      }
    }).finally(() => {
      if (!controller.signal.aborted) setSearching(false);
    });
    return () => controller.abort();
  }, [csrfToken, offset, onSessionExpired, profile, query, searchRevision, serverId, serverMinecraftVersion, serverSoftware]);

  const loadProject = useCallback((hit: MarketplaceSearchHit): void => {
    setSelectedHit(hit);
    setDetail(null);
    setDetailError(null);
    setSelectedVersionId('');
    setBodyExpanded(false);
  }, []);

  useEffect(() => {
    if (selectedHit === null || profile === null) return;
    const controller = new AbortController();
    setDetailLoading(true);
    setDetailError(null);
    void getMarketplaceProject(server.id, selectedHit.projectId, csrfToken, controller.signal).then((next) => {
      if (!marketplaceResponseMatchesServer(next.instanceId, next.compatibility, server) || next.project.id !== selectedHit.projectId) {
        throw new ApiError('Marketplace returned project details for a different server or project.');
      }
      setDetail(next);
      setSelectedVersionId(defaultMarketplaceVersion(next.versions)?.id ?? '');
    }).catch((requestError: unknown) => {
      if (controller.signal.aborted) return;
      if (isSessionError(requestError)) onSessionExpired();
      else setDetailError(describeError(requestError));
    }).finally(() => {
      if (!controller.signal.aborted) setDetailLoading(false);
    });
    return () => controller.abort();
  }, [csrfToken, detailRevision, onSessionExpired, profile, selectedHit, serverId, serverMinecraftVersion, serverSoftware]);

  const selectedVersion = useMemo(() => detail?.versions.find((version) => version.id === selectedVersionId) ?? null, [detail, selectedVersionId]);
  if (profile === null) {
    return <section class="server-tool marketplace-panel"><div class="marketplace-state marketplace-state--error"><Icon name="warning" /><strong>Marketplace unavailable</strong><span>{server.software} servers do not have a Helix marketplace installer.</span></div></section>;
  }
  if (selectedHit !== null) {
    return <section class="server-tool marketplace-panel"><ProjectView hit={selectedHit} detail={detail} loading={detailLoading} error={detailError} canManageServers={canManageServers} selectedVersionId={selectedVersionId} bodyExpanded={bodyExpanded} onVersionChange={setSelectedVersionId} onBodyToggle={() => setBodyExpanded((value) => !value)} onBack={() => { setSelectedHit(null); setDetail(null); setInstallOpen(false); }} onRetry={() => setDetailRevision((value) => value + 1)} onInstall={() => setInstallOpen(true)} />{installOpen && detail !== null && selectedVersion !== null && <InstallDialog server={server} detail={detail} version={selectedVersion} csrfToken={csrfToken} canManageServers={canManageServers} onSessionExpired={onSessionExpired} onInstalled={onInstalled} onClose={() => setInstallOpen(false)} />}</section>;
  }
  const firstResult = page === null || page.totalHits === 0 ? 0 : page.offset + 1;
  const lastResult = page === null ? 0 : Math.min(page.offset + page.hits.length, page.totalHits);
  const canGoNext = page !== null && lastResult < page.totalHits && page.offset + page.limit <= 10_000;
  return (
    <section class="server-tool marketplace-panel" ref={searchTop}>
      <header class="marketplace-head"><div><span class="eyebrow">CURATED FROM MODRINTH</span><h2>{profile.contentKind === 'plugin' ? 'Plugin marketplace' : 'Mod marketplace'}</h2><p>Browse only server-side content compatible with this exact Minecraft runtime.</p></div><a class="marketplace-modrinth" href="https://modrinth.com" target="_blank" rel="noreferrer">Powered by Modrinth <Icon name="external" size={13} /></a></header>
      {page !== null && <CompatibilityBar page={page} />}
      <label class="marketplace-search"><Icon name="search" size={18} /><span class="sr-only">Search compatible projects</span><input type="search" value={draftQuery} maxlength={120} autocomplete="off" placeholder={`Search ${profile.contentKind === 'plugin' ? 'plugins' : 'mods'} by name`} onInput={(event) => setDraftQuery(event.currentTarget.value)} />{draftQuery.length > 0 && <button type="button" onClick={() => setDraftQuery('')} aria-label="Clear marketplace search"><Icon name="close" size={15} /></button>}</label>
      <div class="marketplace-results-head"><div><strong>{page === null ? 'Compatible projects' : `${wholeNumber.format(page.totalHits)} compatible project${page.totalHits === 1 ? '' : 's'}`}</strong><span>{query.length === 0 ? 'Popular results for this server' : `Results for “${query}”`}</span></div>{page !== null && page.totalHits > 0 && <span>{firstResult}–{lastResult} of {wholeNumber.format(page.totalHits)}</span>}</div>
      <SearchResults page={page} loading={searching} error={searchError} onOpen={loadProject} onRetry={() => setSearchRevision((value) => value + 1)} />
      {page !== null && page.totalHits > 0 && <nav class="marketplace-pagination" aria-label="Marketplace result pages"><button class="button button--quiet" type="button" disabled={searching || page.offset === 0} onClick={() => { setOffset(Math.max(0, page.offset - page.limit)); searchTop.current?.scrollIntoView({ behavior: 'smooth', block: 'start' }); }}><Icon name="back" size={14} />Previous</button><span>Showing {firstResult}–{lastResult}</span><button class="button button--quiet" type="button" disabled={searching || !canGoNext} onClick={() => { setOffset(page.offset + page.limit); searchTop.current?.scrollIntoView({ behavior: 'smooth', block: 'start' }); }}>Next<Icon name="chevron" size={14} /></button></nav>}
      <footer class="marketplace-attribution">Results, metadata, and artwork come from <a href="https://modrinth.com" target="_blank" rel="noreferrer">Modrinth <Icon name="external" size={12} /></a>. Artwork is fetched through Helix’s bounded image proxy instead of connecting the browser directly to arbitrary image hosts.</footer>
    </section>
  );
}
