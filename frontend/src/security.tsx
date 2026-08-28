import { useCallback, useEffect, useMemo, useState } from 'preact/hooks';
import { ApiError } from './api';
import { InlineError, PageHead } from './dashboard-ui';
import { Icon } from './icons';
import { InfoTip } from './info-tip';
import { Dialog } from './modal';
import {
  getSecurityInventory,
  setSecurityControl,
  type SecurityControl,
  type SecurityInventory,
} from './security-api';
import './security.css';

export interface SecurityPageProps {
  csrfToken: string;
  canManage: boolean;
  themeLabel: string;
  helixVersion: string | null;
  onSessionExpired: () => void;
}

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : 'Helix could not complete that security change.';
}

function stateTone(control: SecurityControl): 'good' | 'warning' | 'idle' {
  if (control.state === 'always_on' || (control.recommended && control.enabled)) return 'good';
  if (control.recommended && !control.enabled) return 'warning';
  return 'idle';
}

function confirmationFor(control: SecurityControl, next: boolean): string {
  return (next ? control.confirmationEnable : control.confirmationDisable) ?? '';
}

export function SecurityPage({ csrfToken, canManage, themeLabel, helixVersion, onSessionExpired }: SecurityPageProps) {
  const [inventory, setInventory] = useState<SecurityInventory | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState<SecurityControl | null>(null);
  const [typed, setTyped] = useState('');
  const [busy, setBusy] = useState(false);
  const [filter, setFilter] = useState<'recommended' | 'all'>('recommended');

  const refresh = useCallback(async (signal?: AbortSignal): Promise<void> => {
    setLoading(true);
    setError(null);
    try {
      setInventory(await getSecurityInventory(csrfToken, signal));
    } catch (reason) {
      if (reason instanceof ApiError && reason.status === 401) onSessionExpired();
      else if (signal?.aborted !== true) setError(describeError(reason));
    } finally {
      if (signal?.aborted !== true) setLoading(false);
    }
  }, [csrfToken, onSessionExpired]);

  useEffect(() => {
    const controller = new AbortController();
    void refresh(controller.signal);
    return () => controller.abort();
  }, [refresh]);

  const controls = inventory?.controls ?? [];
  const visible = useMemo(
    () => filter === 'all' ? controls : controls.filter((control) => control.recommended || control.writable),
    [controls, filter],
  );

  const apply = async (): Promise<void> => {
    if (pending === null || busy) return;
    const next = !pending.enabled;
    if (typed !== confirmationFor(pending, next)) return;
    setBusy(true);
    setError(null);
    try {
      await setSecurityControl(pending.id, next, typed, csrfToken);
      setPending(null);
      setTyped('');
      await refresh();
    } catch (reason) {
      if (reason instanceof ApiError && reason.status === 401) onSessionExpired();
      else setError(describeError(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="page page--security">
      <PageHead
        title="Security"
        detail="Exact host and Helix controls, with the reason each one exists and what changes if you flip it."
        actions={
          <button class="button button--quiet" type="button" disabled={loading} onClick={() => void refresh()}>
            <Icon name="refresh" size={15} />{loading ? 'Checking…' : 'Recheck'}
          </button>
        }
      />
      <InlineError message={error} />
      <section class="security-identity">
        <div><span>Helix</span><strong>{helixVersion ?? '—'}</strong><small>{themeLabel} theme</small></div>
        <div><span>Kernel</span><strong>{inventory?.facts.kernel ?? '—'}</strong><small>Observed from this host</small></div>
        <div><span>AppArmor</span><strong>{inventory?.facts.apparmor ?? '—'}</strong><small>Linux confinement</small></div>
        <div><span>UFW</span><strong>{inventory?.facts.ufw ?? '—'}</strong><small>Host firewall status</small></div>
      </section>
      <div class="security-toolbar">
        <p>Recommended items are the defaults Helix wants on a private LAN dashboard. Writable switches still require typing an exact confirmation phrase. Helix will not disable UFW, rewrite sshd, or expose a root shell from here.</p>
        <div class="security-filters" role="tablist" aria-label="Security views">
          <button type="button" class={filter === 'recommended' ? 'is-active' : ''} onClick={() => setFilter('recommended')}>Recommended</button>
          <button type="button" class={filter === 'all' ? 'is-active' : ''} onClick={() => setFilter('all')}>Everything</button>
        </div>
      </div>
      <div class="security-list">
        {visible.map((control) => (
          <article class="security-card surface" key={control.id}>
            <header>
              <div>
                <span class={`state-label state-label--${stateTone(control)}`}>{control.state.replaceAll('_', ' ')}</span>
                {control.recommended && <em>Recommended on</em>}
              </div>
              <h2>{control.title}</h2>
            </header>
            <p>{control.summary}</p>
            <details>
              <summary>Why this exists <InfoTip text="Open this for the edge cases and the cost of leaving it off." /></summary>
              <p>{control.implications}</p>
              <p><strong>If you would turn it off:</strong> {control.offReason}</p>
            </details>
            {control.writable ? (
              <button
                class="switch-button"
                role="switch"
                type="button"
                disabled={!canManage || busy}
                aria-checked={control.enabled}
                onClick={() => { setPending(control); setTyped(''); }}
              >
                <i />
                <span>{control.enabled ? 'On' : 'Off'}</span>
              </button>
            ) : (
              <small class="security-locked"><Icon name="info" size={13} />{control.state === 'always_on' ? 'Compiled into Helix. Not a switch.' : 'Observed only. Change this on the host if you need to.'}</small>
            )}
          </article>
        ))}
      </div>
      {pending !== null && (
        <Dialog title={`${pending.enabled ? 'Turn off' : 'Turn on'} ${pending.title}?`} onClose={() => setPending(null)}>
          <p class="security-confirm">{pending.enabled ? pending.offReason : pending.summary}</p>
          <p class="security-confirm">{pending.implications}</p>
          <label class="field">
            <span>Type <code>{confirmationFor(pending, !pending.enabled)}</code></span>
            <input value={typed} autocomplete="off" autocapitalize="none" spellcheck={false} onInput={(event) => setTyped(event.currentTarget.value)} />
          </label>
          <div class="dialog-actions">
            <button class="button button--quiet" type="button" disabled={busy} onClick={() => setPending(null)}>Cancel</button>
            <button class="button button--primary" type="button" disabled={busy || typed !== confirmationFor(pending, !pending.enabled)} onClick={() => void apply()}>{busy ? 'Saving…' : pending.enabled ? 'Turn off' : 'Turn on'}</button>
          </div>
        </Dialog>
      )}
    </div>
  );
}
