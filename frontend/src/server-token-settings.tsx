import { useState } from 'preact/hooks';
import { expectArray, expectNumber, expectRecord, expectString, requestJson } from './api';
import type { ManagedServer } from './control-api';
import { InlineError } from './dashboard-ui';
import { Icon } from './icons';
import './server-token-settings.css';

interface Token { id: string; name: string; servers: string[]; permissions: string[]; createdAt: number | null; expiresAt: number | null; lastUsedAt: number | null; revoked: boolean; authorized: boolean }
const EXPIRY_DAYS = [1, 7, 30, 90, 180, 365];
function optionalTime(t: Record<string, unknown>, key: string): number | null { return t[key] === null || t[key] === undefined ? null : expectNumber(t, key, 'token'); }
function when(ms: number): string { return new Date(ms).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }); }
function expiryLabel(days: number): string { return days === 365 ? '1 year' : days === 1 ? '1 day' : `${days} days`; }
export function catalog(value: unknown): { tokens: Token[]; permissions: string[] } {
  const data = expectRecord(value, 'API tokens');
  return {
    permissions: expectArray(data, 'permissions', 'API tokens', 64).map((p) => expectString({ value: p }, 'value', 'permission')),
    tokens: expectArray(data, 'tokens', 'API tokens', 256).map((value) => {
      const t = expectRecord(value, 'token');
      return { id: expectString(t, 'id', 'token'), name: expectString(t, 'name', 'token'), servers: expectArray(t, 'servers', 'token', 64).map((s) => expectString({ value: s }, 'value', 'server')), permissions: expectArray(t, 'permissions', 'token', 64).map((p) => expectString({ value: p }, 'value', 'permission')), createdAt: optionalTime(t, 'created_at'), expiresAt: optionalTime(t, 'expires_at'), lastUsedAt: optionalTime(t, 'last_used_at'), revoked: t.revoked_at !== null && t.revoked_at !== undefined, authorized: t.authorized !== false };
    }),
  };
}
export function tokenStatus(t: Token, now: number): { label: string; usable: boolean } {
  if (t.revoked) return { label: 'Revoked', usable: false };
  if (t.expiresAt !== null && t.expiresAt <= now) return { label: 'Expired', usable: false };
  if (!t.authorized) return { label: 'Invalid — your account or password changed', usable: false };
  return { label: t.expiresAt === null ? 'Active · never expires' : `Active · expires ${when(t.expiresAt)}`, usable: true };
}

export function ServerTokenSettings({ csrfToken, servers }: { csrfToken: string; servers: ManagedServer[] }) {
  const [data, setData] = useState<ReturnType<typeof catalog> | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [days, setDays] = useState<number | null>(30);
  const [selected, setSelected] = useState<string[]>([]);
  const [permissions, setPermissions] = useState<string[]>(['view']);
  const [secret, setSecretValue] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [confirmRevoke, setConfirmRevoke] = useState<string | null>(null);
  const [confirmRotate, setConfirmRotate] = useState<string | null>(null);
  function setSecret(value: string | null) { setSecretValue(value); setCopied(false); }
  async function refresh() { setData(await requestJson('/api/v1/auth/server-tokens', catalog, { csrfToken })); }
  async function run(operation: () => Promise<void>) {
    if (busy) return;
    setBusy(true); setError(null);
    try { await operation(); } catch (e) { setError(e instanceof Error ? e.message : 'The request failed. Refresh the token list before trying again.'); }
    finally { setBusy(false); }
  }
  function toggle(list: string[], id: string): string[] { return list.includes(id) ? list.filter((x) => x !== id) : [...list, id]; }
  function serverName(id: string): string { return servers.find((s) => s.id === id)?.name ?? id; }
  return <section class="settings-card server-token-settings">
    <div class="settings-card__head"><div><Icon name="servers" /><span><h2>Server API tokens</h2><p>Give a tool access to selected servers without sharing your login.</p></span></div></div>
    {error !== null && <div class="server-token-settings__error"><InlineError message={error} /></div>}
    {data === null ? <div class="server-token-settings__intro">
      <p>Create, check, rotate, and revoke tokens for scripts and plugins.</p>
      <button class="button button--quiet" type="button" disabled={busy} onClick={() => void run(refresh)}>{busy ? 'Loading…' : 'Manage tokens'}</button>
    </div> : <>
      {secret !== null && <div class="server-token-settings__secret" role="status">
        <strong>New token — copy it now</strong>
        <small>Helix stores only a verifier, so this exact value is not shown again. If you lose it, use Rotate on the token to get a new one.</small>
        <div class="server-token-settings__secret-row">
          <input aria-label="New API token" readOnly value={secret} autoComplete="off" spellcheck={false} onFocus={(e) => e.currentTarget.select()} />
          <button class="button button--primary" type="button" onClick={() => { void navigator.clipboard?.writeText(secret).then(() => setCopied(true), () => setCopied(false)); }}>{copied ? 'Copied' : 'Copy'}</button>
          <button class="button button--quiet" type="button" onClick={() => setSecret(null)}>Hide</button>
        </div>
      </div>}
      <form class="server-token-settings__form" onSubmit={(e) => { e.preventDefault(); void run(async () => {
        setSecret(null);
        const created = await requestJson('/api/v1/auth/server-tokens', (value) => expectString(expectRecord(value, 'new token'), 'token', 'new token'), { csrfToken, method: 'POST', body: { name: name.trim(), servers: selected, permissions, expires_in_days: days } });
        setSecret(created); setName(''); await refresh();
      }); }}>
        <h3>New token</h3>
        <div class="server-token-settings__fields">
          <label><span>Token name</span><input required maxLength={80} value={name} onInput={(e) => setName(e.currentTarget.value)} placeholder="Plugin deployment" /></label>
          <label><span>Expires after</span><select value={days === null ? 'never' : days} onChange={(e) => setDays(e.currentTarget.value === 'never' ? null : Number(e.currentTarget.value))}>{EXPIRY_DAYS.map((d) => <option key={d} value={d}>{expiryLabel(d)}</option>)}<option value="never">Never (until revoked)</option></select></label>
        </div>
        <fieldset disabled={busy}><legend>Servers</legend>
          {servers.length === 0 && <p class="server-token-settings__empty">No servers available.</p>}
          <div class="server-token-settings__choices server-token-settings__choices--servers">{servers.map((s) => <label key={s.id}><input type="checkbox" checked={selected.includes(s.id)} onChange={() => setSelected(toggle(selected, s.id))} /><span><strong>{s.name}</strong><small>{s.id}</small></span></label>)}</div>
        </fieldset>
        <fieldset disabled={busy}><legend>Permissions</legend>
          <div class="server-token-settings__choices">{data.permissions.map((p) => <label key={p}><input type="checkbox" checked={permissions.includes(p)} onChange={() => setPermissions(toggle(permissions, p))} /><span><strong>{p}</strong></span></label>)}</div>
        </fieldset>
        <div class="server-token-settings__form-actions">
          <span>Use HTTPS or an SSH tunnel. Grant only what the tool needs; tokens can't administer the host.</span>
          <button class="button button--primary" type="submit" disabled={busy || name.trim() === '' || selected.length === 0 || permissions.length === 0}>{busy ? 'Working…' : 'Create token'}</button>
        </div>
      </form>
      <div class="server-token-settings__list">
        <h3>Your tokens <small>{data.tokens.length}</small></h3>
        {data.tokens.length === 0 && <p class="server-token-settings__empty">No tokens yet.</p>}
        {data.tokens.map((t) => { const status = tokenStatus(t, Date.now()); return <article key={t.id} class={status.usable ? '' : 'is-inactive'}>
          <div class="server-token-settings__token">
            <div class="server-token-settings__token-title"><strong>{t.name}</strong><span class={`server-token-settings__status ${status.usable ? 'is-ok' : 'is-bad'}`}>{status.label}</span></div>
            <dl>
              <dt>Servers</dt><dd>{t.servers.map(serverName).join(', ')}</dd>
              <dt>Permissions</dt><dd>{t.permissions.join(', ')}</dd>
              <dt>Created</dt><dd>{t.createdAt === null ? '—' : when(t.createdAt)}</dd>
              <dt>Last used</dt><dd>{t.lastUsedAt === null ? 'Never' : when(t.lastUsedAt)}</dd>
            </dl>
          </div>
          {!t.revoked && <div class="server-token-settings__actions">
            {confirmRotate === t.id ? <><small>The current value stops working immediately.</small><button class="button button--primary" type="button" disabled={busy} onClick={() => void run(async () => {
              const next = await requestJson(`/api/v1/auth/server-tokens/${encodeURIComponent(t.id)}/rotate`, (v) => expectString(expectRecord(v, 'rotated token'), 'token', 'rotated token'), { csrfToken, method: 'POST', body: {} });
              setSecret(next); setConfirmRotate(null); await refresh();
            })}>Confirm rotate</button><button class="button button--quiet" type="button" disabled={busy} onClick={() => setConfirmRotate(null)}>Cancel</button></>
            : confirmRevoke === t.id ? <><small>Tools using this token lose access.</small><button class="button button--danger" type="button" disabled={busy} onClick={() => void run(async () => { await requestJson(`/api/v1/auth/server-tokens/${encodeURIComponent(t.id)}`, (v) => expectRecord(v, 'revocation'), { csrfToken, method: 'DELETE', body: {} }); setConfirmRevoke(null); await refresh(); })}>Confirm revoke</button><button class="button button--quiet" type="button" disabled={busy} onClick={() => setConfirmRevoke(null)}>Cancel</button></>
            : <><button class="button button--quiet" type="button" disabled={busy || !status.usable} title="Issue a new value for this token and show it once" onClick={() => { setConfirmRevoke(null); setConfirmRotate(t.id); }}>Rotate &amp; show</button><button class="button button--quiet" type="button" disabled={busy} onClick={() => { setConfirmRotate(null); setConfirmRevoke(t.id); }}>Revoke</button></>}
          </div>}
        </article>; })}
      </div>
      <div class="settings-card__foot"><span>Tokens are stored as verifiers only</span><button class="button button--quiet" type="button" disabled={busy} onClick={() => void run(refresh)}>Refresh</button></div>
    </>}
  </section>;
}
