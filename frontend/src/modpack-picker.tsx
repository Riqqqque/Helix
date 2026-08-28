import { useEffect, useMemo, useState } from 'preact/hooks';
import { ApiError } from './api';
import { formatBytes } from './format';
import { Icon } from './icons';
import {
  getModpackProject,
  searchModpacks,
  type ModpackProjectDetail,
  type ModpackProvider,
  type ModpackSearchPage,
  type ModpackSearchResult,
  type ModpackSelection,
} from './modpack-api';
import './modpack.css';

const PAGE_SIZE = 12;

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : 'Helix could not reach the modpack catalog.';
}

function isSessionError(error: unknown): boolean {
  return error instanceof ApiError && (error.status === 401 || error.code === 'csrf_rejected');
}

function compactNumber(value: number): string {
  return new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 }).format(value);
}

function ModpackMark({ iconUrl, size = 20 }: { iconUrl: string | null; size?: number }) {
  const [imageFailed, setImageFailed] = useState(false);
  useEffect(() => setImageFailed(false), [iconUrl]);
  return <span class="modpack-mark">{iconUrl !== null && !imageFailed ? <img src={iconUrl} alt="" loading="lazy" decoding="async" onError={() => setImageFailed(true)} /> : <Icon name="servers" size={size} />}</span>;
}

export interface ModpackPickerProps {
  csrfToken: string;
  selection: ModpackSelection | null;
  onSelectionChange: (selection: ModpackSelection | null) => void;
  onSessionExpired: () => void;
}

function candidateLabel(status: string): string {
  if (status === 'forge_candidate') return 'Forge';
  if (status === 'neoforge_candidate') return 'NeoForge';
  if (status === 'quilt_candidate') return 'Quilt';
  if (status === 'fabric_candidate') return 'Fabric';
  return 'Preview';
}

export function ModpackPicker({ csrfToken, selection, onSelectionChange, onSessionExpired }: ModpackPickerProps) {
  const [query, setQuery] = useState('');
  const [offset, setOffset] = useState(0);
  const [provider, setProvider] = useState<ModpackProvider>('modrinth');
  const [page, setPage] = useState<ModpackSearchPage | null>(null);
  const [detail, setDetail] = useState<ModpackProjectDetail | null>(null);
  const [selectedVersionId, setSelectedVersionId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [detailLoading, setDetailLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);

  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setError(null);
    const timer = window.setTimeout(() => {
      void searchModpacks(query.trim(), offset, PAGE_SIZE, csrfToken, controller.signal, provider)
        .then((result) => setPage(result))
        .catch((requestError: unknown) => {
          if (controller.signal.aborted) return;
          setPage(null);
          if (isSessionError(requestError)) onSessionExpired();
          else setError(errorMessage(requestError));
        })
        .finally(() => {
          if (!controller.signal.aborted) setLoading(false);
        });
    }, 350);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [csrfToken, offset, onSessionExpired, provider, query, retry]);

  const openProject = async (project: ModpackSearchResult): Promise<void> => {
    setDetailLoading(true);
    setError(null);
    setDetail(null);
    setSelectedVersionId(null);
    try {
      const next = await getModpackProject(project.projectId, csrfToken);
      setDetail(next);
      setSelectedVersionId(next.versions.find((version) => version.installable)?.id ?? null);
    } catch (requestError) {
      if (isSessionError(requestError)) onSessionExpired();
      else setError(errorMessage(requestError));
    } finally {
      setDetailLoading(false);
    }
  };

  const selectedVersion = useMemo(
    () => detail?.versions.find((version) => version.id === selectedVersionId) ?? null,
    [detail, selectedVersionId],
  );

  const choose = (): void => {
    if (detail === null || selectedVersion?.installable !== true || selectedVersion.mrpackFile === null) return;
    onSelectionChange({
      projectId: detail.project.id,
      projectSlug: detail.project.slug,
      projectTitle: detail.project.title,
      projectWebUrl: detail.project.webUrl,
      versionId: selectedVersion.id,
      versionName: selectedVersion.name,
      versionNumber: selectedVersion.versionNumber,
      minecraftVersions: selectedVersion.gameVersions,
      filename: selectedVersion.mrpackFile.filename,
      fileSize: selectedVersion.mrpackFile.size,
      provider,
    });
  };

  if (detail !== null) {
    return (
      <section class="modpack-detail" aria-busy={detailLoading}>
        <header class="modpack-detail__head">
          <button class="button button--quiet" type="button" onClick={() => setDetail(null)}><Icon name="chevron" size={14} />Back to results</button>
          <a href={detail.project.webUrl} target="_blank" rel="noreferrer">Open catalog <Icon name="external" size={12} /></a>
        </header>
        <div class="modpack-detail__identity">
          <ModpackMark iconUrl={detail.project.iconUrl} size={24} />
          <div>
            <small>{provider === 'curseforge' ? 'CurseForge modpack' : 'Modrinth modpack'}</small>
            <h3>{detail.project.title}</h3>
            <p>{detail.project.description ?? 'No short description was provided.'}</p>
            <span>{compactNumber(detail.project.downloads)} downloads · {detail.compatibleVersionCount} installable {detail.compatibleVersionCount === 1 ? 'release' : 'releases'}</span>
          </div>
        </div>
        <div class="modpack-compatibility-note">
          <Icon name="info" size={16} />
          <span><strong>Dedicated server install</strong>Helix downloads the selected pack, pins a matching loader, and starts the isolated server. Client-only extras stay out. This is not a full client pack copy.</span>
        </div>
        <div class="modpack-version-layout">
          <fieldset class="modpack-versions">
            <legend>Compatible version</legend>
            {detail.versions.map((version) => (
              <label class={`${version.installable ? '' : 'is-disabled'}${selectedVersionId === version.id ? ' is-selected' : ''}`} key={version.id}>
                <input type="radio" name="modpack-version" disabled={!version.installable} checked={selectedVersionId === version.id} onChange={() => setSelectedVersionId(version.id)} />
                <span>
                  <strong>{version.versionNumber}<small>{version.versionType}</small></strong>
                  <span>{version.gameVersions.slice(0, 4).join(', ') || 'No Minecraft version'} · {version.loaders.join(', ') || 'No loader'}</span>
                  <small>{version.compatibilityReason}</small>
                </span>
              </label>
            ))}
            {detail.versions.length === 0 && <div class="modpack-empty">This project has no published versions.</div>}
          </fieldset>
          <aside class="modpack-version-summary">
            <small>Selected release</small>
            <strong>{selectedVersion?.name ?? 'No installable server release'}</strong>
            {selectedVersion?.mrpackFile !== null && selectedVersion?.mrpackFile !== undefined && <>
              <span>{selectedVersion.mrpackFile.filename}</span>
              <span>{formatBytes(selectedVersion.mrpackFile.size)}</span>
            </>}
            <p>Latest listed release is selected by default. Helix re-resolves the opaque project and version IDs before it downloads anything.</p>
            <button class="button button--primary" type="button" disabled={selectedVersion?.installable !== true} onClick={choose}>{selection?.projectId === detail.project.id && selection.versionId === selectedVersionId ? 'Selected' : 'Use this modpack'}</button>
          </aside>
        </div>
        {detail.project.body !== null && <details class="modpack-description"><summary>Project description</summary><p>{detail.project.body}</p></details>}
      </section>
    );
  }

  return (
    <section class="modpack-browser">
      <div class="modpack-browser__intro">
        <div><strong>Start with a modpack</strong><span>Browse server-capable packs without leaving Helix. Switch catalogs without giving Helix an API key.</span></div>
        <a href={provider === 'curseforge' ? 'https://www.curseforge.com/minecraft/modpacks' : 'https://modrinth.com/modpacks'} target="_blank" rel="noreferrer">{provider === 'curseforge' ? 'CurseForge catalog' : 'Modrinth catalog'} <Icon name="external" size={12} /></a>
      </div>
      <div class="modpack-provider-tabs" role="tablist" aria-label="Modpack catalog">
        <button type="button" class={provider === 'modrinth' ? 'is-active' : ''} aria-selected={provider === 'modrinth'} onClick={() => { setProvider('modrinth'); setOffset(0); setPage(null); }}>Modrinth</button>
        <button type="button" class={provider === 'curseforge' ? 'is-active' : ''} aria-selected={provider === 'curseforge'} onClick={() => { setProvider('curseforge'); setOffset(0); setPage(null); }}>CurseForge</button>
      </div>
      {selection !== null && <div class="modpack-selection"><Icon name="check" size={16} /><span><strong>{selection.projectTitle}</strong>{selection.versionNumber} · {selection.minecraftVersions.join(', ')}</span><button type="button" onClick={() => onSelectionChange(null)}>Clear</button></div>}
      <label class="modpack-search"><span>Search packs</span><div><Icon name="search" size={16} /><input value={query} onInput={(event) => { setQuery(event.currentTarget.value); setOffset(0); }} maxlength={120} placeholder="All the Mods, Cobblemon, adventure…" /></div></label>
      {error !== null && <div class="modpack-state is-error" role="alert"><Icon name="warning" /><span>{error}</span><button type="button" onClick={() => setRetry((value) => value + 1)}>Try again</button></div>}
      {loading && <div class="modpack-state" role="status"><span class="modpack-spinner" /><span>Searching the {provider === 'curseforge' ? 'CurseForge' : 'Modrinth'} catalog…</span></div>}
      {!loading && error === null && page?.results.length === 0 && <div class="modpack-state"><Icon name="search" /><span>No modpacks match this search.</span></div>}
      <div class="modpack-grid" aria-live="polite">
        {!loading && page?.results.map((project) => (
          <article class={project.compatibilityStatus === 'incompatible' ? 'is-incompatible' : ''} key={project.projectId}>
            <header><ModpackMark iconUrl={project.iconUrl} /><span class={`modpack-status is-${project.compatibilityStatus}`}>{candidateLabel(project.compatibilityStatus)}</span></header>
            <div><h3>{project.title}</h3><span>by {project.author ?? (provider === 'curseforge' ? 'CurseForge creator' : 'Modrinth creator')}</span></div>
            <p>{project.description ?? 'No short description was provided.'}</p>
            <small>{project.compatibilityReason}</small>
            <footer><span>{compactNumber(project.downloads)} downloads</span><button type="button" disabled={detailLoading} onClick={() => void openProject(project)}>{detailLoading ? 'Loading…' : 'View releases'}</button></footer>
          </article>
        ))}
      </div>
      {page !== null && page.totalHits > PAGE_SIZE && <nav class="modpack-pagination" aria-label="Modpack search pages"><button type="button" disabled={offset === 0 || loading} onClick={() => setOffset(Math.max(0, offset - PAGE_SIZE))}>Previous</button><span>{offset + 1}–{Math.min(offset + PAGE_SIZE, page.totalHits)} of {page.totalHits}</span><button type="button" disabled={offset + PAGE_SIZE >= page.totalHits || loading} onClick={() => setOffset(offset + PAGE_SIZE)}>Next</button></nav>}
    </section>
  );
}
