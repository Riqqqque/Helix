import { useCallback, useEffect, useState } from 'preact/hooks';
import { ApiError } from './api';
import { InlineError, PageHead } from './dashboard-ui';
import { Icon } from './icons';
import { InfoTip } from './info-tip';
import { Dialog } from './modal';
import { StrandFrame } from './strand-frame';
import {
  deleteStrand,
  downloadStrandPackage,
  fileToBase64,
  inspectStrand,
  installStrand,
  listStrands,
  setStrandEnabled,
  type StrandCapability,
  type StrandInspect,
  type StrandSummary,
} from './strands-api';
import './strands.css';

export interface StrandsPageProps {
  csrfToken: string;
  canManage: boolean;
  onSessionExpired: () => void;
}

function selectedStrandId(): string | null {
  const hash = window.location.hash.startsWith('#') ? window.location.hash.slice(1) : window.location.hash;
  const match = /^strands\/([0-9a-f-]{36})$/u.exec(hash);
  return match?.[1] ?? null;
}

function capabilityLabel(capability: StrandCapability): string {
  if (capability.origins.length === 0) return capability.name;
  return `${capability.name} (${capability.origins.join(', ')})`;
}

export function StrandsPage({ csrfToken, canManage, onSessionExpired }: StrandsPageProps) {
  const [strands, setStrands] = useState<StrandSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [url, setUrl] = useState('');
  const [preview, setPreview] = useState<StrandInspect | null>(null);
  const [pendingSource, setPendingSource] = useState<{ source: 'upload'; filename: string; bytesBase64: string } | { source: 'url'; url: string } | null>(null);
  const [openId, setOpenId] = useState<string | null>(() => typeof window === 'undefined' ? null : selectedStrandId());
  const [dropping, setDropping] = useState(false);

  const load = useCallback(async (): Promise<void> => {
    const next = await listStrands(csrfToken);
    setStrands(next);
    setError(null);
  }, [csrfToken]);

  useEffect(() => {
    const update = (): void => setOpenId(selectedStrandId());
    window.addEventListener('hashchange', update);
    return () => window.removeEventListener('hashchange', update);
  }, []);

  useEffect(() => {
    let mounted = true;
    void load().catch((caught: unknown) => {
      if (!mounted) return;
      if (caught instanceof ApiError && (caught.status === 401 || caught.code === 'csrf_rejected')) {
        onSessionExpired();
        return;
      }
      setError(caught instanceof Error ? caught.message : 'Strands could not be loaded.');
    });
    return () => { mounted = false; };
  }, [load, onSessionExpired]);

  const openStrand = strands.find((strand) => strand.id === openId) ?? null;

  const run = async (work: () => Promise<void>): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      await work();
      await load();
    } catch (caught: unknown) {
      if (caught instanceof ApiError && (caught.status === 401 || caught.code === 'csrf_rejected')) {
        onSessionExpired();
        return;
      }
      setError(caught instanceof Error ? caught.message : 'Helix could not complete that Strand action.');
    } finally {
      setBusy(false);
    }
  };

  const reviewFile = async (file: File): Promise<void> => {
    const bytesBase64 = await fileToBase64(file);
    const source = { source: 'upload' as const, filename: file.name, bytesBase64 };
    const inspected = await inspectStrand(csrfToken, source);
    setPendingSource(source);
    setPreview(inspected);
  };

  const reviewUrl = async (): Promise<void> => {
    const source = { source: 'url' as const, url: url.trim() };
    const inspected = await inspectStrand(csrfToken, source);
    setPendingSource(source);
    setPreview(inspected);
  };

  return (
    <div class="page page--strands">
      <PageHead
        title="Strands"
        detail="Install someone else's zip, or pack your own with helixctl strand pack. Helix reviews capabilities before a Strand can run."
      />
      <InlineError message={error} />

      {openStrand !== null && openStrand.enabled ? (
        <section class="strand-runtime">
          <div class="strand-runtime__bar">
            <a href="#strands"><Icon name="back" size={16} /> All Strands</a>
            <strong>{openStrand.name}</strong>
            <span>{openStrand.version}</span>
          </div>
          <StrandFrame
            strandId={openStrand.id}
            uiEntry={openStrand.uiEntry}
            csrfToken={csrfToken}
            title={openStrand.name}
            onSessionExpired={onSessionExpired}
          />
        </section>
      ) : (
        <>
          {canManage && (
            <section
              class={`strands-install${dropping ? ' is-dropping' : ''}`}
              onDragEnter={(event) => {
                event.preventDefault();
                if (busy) return;
                setDropping(true);
              }}
              onDragOver={(event) => {
                event.preventDefault();
                if (event.dataTransfer !== null) event.dataTransfer.dropEffect = busy ? 'none' : 'copy';
              }}
              onDragLeave={(event) => {
                if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
                setDropping(false);
              }}
              onDrop={(event) => {
                event.preventDefault();
                setDropping(false);
                const file = event.dataTransfer?.files[0];
                if (file && !busy) void run(() => reviewFile(file));
              }}
            >
              <div>
                <h2>Install a Strand</h2>
                <p>Drop a <code>.strand.zip</code>, choose a file, or paste an https zip URL. Helix shows the exact host calls before anything is enabled.</p>
              </div>
              {dropping && <div class="strands-install__drop" aria-hidden="true">Drop the zip to review it</div>}
              <label class="button button--primary">
                Choose zip
                <input type="file" accept=".zip,application/zip" hidden disabled={busy} onChange={(event) => {
                  const file = event.currentTarget.files?.[0];
                  event.currentTarget.value = '';
                  if (file) void run(() => reviewFile(file));
                }} />
              </label>
              <div class="strands-install__url">
                <input value={url} placeholder="https://example.com/vacuum.strand.zip" disabled={busy} onInput={(event) => setUrl(event.currentTarget.value)} />
                <button class="button" type="button" disabled={busy || url.trim().length === 0} onClick={() => void run(reviewUrl)}>Review URL</button>
              </div>
            </section>
          )}

          <div class="strands-toolbar">
            <div>
              <strong>{strands.length} installed</strong>
              <i />
              <span>{strands.filter((strand) => strand.enabled).length} enabled</span>
            </div>
            <p>
              Strands are isolated UI packages. They cannot call helix-privd, open a shell, or fetch the public internet except through allowlisted HTTPS origins you approve.
              <InfoTip text="Portable Wasm and native sidecars are still not a runtime. If you need a vacuum, printer, or battery API, declare helix:net.https with that exact origin." />
            </p>
          </div>

          {strands.length === 0 ? (
            <section class="strands-empty">
              <Icon name="strands" size={28} />
              <h2>No Strands yet</h2>
              <p>Pack a project with <code>helixctl strand new</code> and <code>helixctl strand pack</code>, then install the zip here.</p>
            </section>
          ) : (
            <ul class="strands-list">
              {strands.map((strand) => (
                <li key={strand.id} class={strand.enabled ? 'is-enabled' : undefined}>
                  <div>
                    <strong>{strand.name}</strong>
                    <small>{strand.slug} · {strand.version} · {strand.publisher}</small>
                    <p>{strand.description}</p>
                    <ul class="strand-caps">
                      {strand.capabilities.map((capability) => (
                        <li key={capability.name}>{capabilityLabel(capability)}</li>
                      ))}
                    </ul>
                  </div>
                  <div class="strand-actions">
                    {strand.enabled && (
                      <a class="button button--primary" href={`#strands/${strand.id}`}>Open</a>
                    )}
                    {canManage && (
                      <button class="button" type="button" disabled={busy} onClick={() => void run(async () => { await setStrandEnabled(csrfToken, strand.id, !strand.enabled); })}>
                        {strand.enabled ? 'Disable' : 'Enable'}
                      </button>
                    )}
                    <button class="button button--quiet" type="button" disabled={busy} onClick={() => void downloadStrandPackage(csrfToken, strand.id, `${strand.slug}.strand.zip`)}>
                      Export zip
                    </button>
                    {canManage && (
                      <button class="button button--quiet" type="button" disabled={busy} onClick={() => {
                        if (window.confirm(`Remove ${strand.name}? Its namespaced storage is deleted too.`)) {
                          void run(async () => { await deleteStrand(csrfToken, strand.id); });
                        }
                      }}>
                        Remove
                      </button>
                    )}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </>
      )}

      {preview !== null && pendingSource !== null && (
        <Dialog title={preview.alreadyInstalled ? `Replace ${preview.name}?` : `Install ${preview.name}?`} onClose={() => { setPreview(null); setPendingSource(null); }}>
          <p>{preview.description}</p>
          <p><strong>{preview.publisher}</strong> · {preview.license} · {preview.version}</p>
          <p>Digest <code>{preview.digestSha256}</code></p>
          {preview.alreadyInstalled && (
            <p>This updates the installed copy{preview.installedVersion ? ` (currently ${preview.installedVersion})` : ''}. Namespaced storage stays. Helix disables it until you Enable again.</p>
          )}
          <h3>Requested host calls</h3>
          {preview.capabilities.length === 0 ? <p>None. This Strand can only render its own UI.</p> : (
            <ul>
              {preview.capabilities.map((capability) => (
                <li key={capability.name}>
                  <strong>{capabilityLabel(capability)}</strong>
                  <span>{capability.reason}</span>
                </li>
              ))}
            </ul>
          )}
          <h3>Files</h3>
          <ul>
            {preview.files.map((file) => (
              <li key={file.path}>{file.path} · {file.bytes} bytes</li>
            ))}
          </ul>
          <p>Installing does not enable it. Review this list, install, then Enable.</p>
          <div class="dialog-actions">
            <button class="button" type="button" onClick={() => { setPreview(null); setPendingSource(null); }}>Cancel</button>
            <button class="button button--primary" type="button" disabled={busy} onClick={() => {
              const source = pendingSource;
              void run(async () => {
                await installStrand(csrfToken, source);
                setPreview(null);
                setPendingSource(null);
              });
            }}>{preview.alreadyInstalled ? 'Replace and disable' : 'Install disabled'}</button>
          </div>
        </Dialog>
      )}
    </div>
  );
}
