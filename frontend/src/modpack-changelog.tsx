import { useEffect, useState } from 'preact/hooks';
import { ApiError, expectRecord, requestJson } from './api';
import type { NativeInstalledModpack } from './control-api';
import type { ModpackVersion } from './modpack-api';
import { Dialog } from './modal';
import { Icon } from './icons';
import { renderMarketplaceBody } from './marketplace-markdown';

export function parseModpackChangelog(value: unknown) {
  const root = expectRecord(value, 'modpack changelog');
  const body = root.body;
  if (typeof body !== 'string' || body.length > 400_000 || (root.format !== 'html' && root.format !== 'markdown') || typeof root.truncated !== 'boolean') throw new Error('Invalid release notes');
  return {body, format: root.format as 'html' | 'markdown', truncated: root.truncated};
}

export function ModpackChangelogButton({pack, version, csrfToken, onSessionExpired}: {
  pack: NativeInstalledModpack; version: ModpackVersion; csrfToken: string; onSessionExpired: () => void;
}) {
  const [open, setOpen] = useState(false);
  return <>
    <button class="button button--quiet" type="button" onClick={() => setOpen(true)}><Icon name="info" size={15} /> Changelog</button>
    {open && <ModpackChangelogDialog pack={pack} version={version} csrfToken={csrfToken} onSessionExpired={onSessionExpired} onClose={() => setOpen(false)} />}
  </>;
}

export function ModpackChangelogDialog({pack, version, csrfToken, onSessionExpired, onClose}: {
  pack: NativeInstalledModpack; version: ModpackVersion; csrfToken: string; onSessionExpired: () => void; onClose: () => void;
}) {
  const [notes, setNotes] = useState<ReturnType<typeof parseModpackChangelog> | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    setNotes(null);
    setError(null);
    const url = `/api/v1/servers/minecraft/modpacks/projects/${encodeURIComponent(pack.projectId)}/versions/${encodeURIComponent(version.id)}/changelog?provider=${pack.provider}`;
    void requestJson(url, parseModpackChangelog, {csrfToken, signal:controller.signal, timeoutMs:95_000}).then(value => {
      if (!controller.signal.aborted) setNotes(value);
    }).catch(failure => {
      if (controller.signal.aborted) return;
      setError(failure instanceof Error ? failure.message : 'Could not load release notes.');
      if (failure instanceof ApiError && failure.status === 401) onSessionExpired();
    });
    return () => controller.abort();
  }, [pack.projectId, pack.provider, version.id, csrfToken, onSessionExpired, retry]);
  const published = version.datePublished ? new Date(version.datePublished) : null;
  return <Dialog title="Changelog" onClose={onClose} wide>
    <div class="modpack-changelog-heading"><strong>{pack.projectTitle}</strong><span>{pack.versionNumber} → {version.versionNumber}</span><small>{pack.provider === 'curseforge' ? 'CurseForge' : 'Modrinth'}{published && Number.isFinite(published.getTime()) ? ` · ${published.toLocaleDateString()}` : ''}</small></div>
    <div class="modpack-changelog-body" aria-busy={!notes && !error}>
      {error ? <div role="alert"><p>{error}</p><button class="button button--quiet" type="button" onClick={() => setRetry(value => value + 1)}><Icon name="refresh" size={15} /> Retry</button></div> : !notes ? <p role="status">Loading release notes…</p> : notes.body.trim() ? renderMarketplaceBody(notes.body, notes.format) : <p>No release notes were published for this version.</p>}
      {notes?.truncated && <p class="modpack-changelog-truncated">These release notes were shortened because of their size.</p>}
    </div>
    <div class="dialog-actions"><button class="button button--quiet" type="button" onClick={onClose}>Close</button></div>
  </Dialog>;
}
