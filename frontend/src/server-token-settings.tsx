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
    <InlineError message={error} />
    {data === null ? <button class="button button--quiet" disabled={busy} onClick={() => void run(refresh)}>{busy ? 'Loading…' : 'Manage tokens'}</button> : <>
      <p class="server-token-settings__warning">Use HTTPS or an SSH tunnel. Grant only the servers and actions your tool needs. Tokens cannot administer the host or create other tokens. A never-expiring token stays valid until you revoke it.</p>
      {secret !== null && <div class="server-token-settings__secret" role="status"><strong>New token — save it before leaving this page.</strong><small>Helix stores only a verifier, so this exact value is not shown again. If you lose it, use Rotate on the token to get a new one.</small><input aria-label="New API token" readOnly value={secret} autoComplete="off" spellcheck={false} onFocus={(e) => e.currentTarget.select()} /><div class="server-token-settings__secret-actions"><button class="button button--quiet" type="button" onClick={() => { void navigator.clipboard?.writeText(secret).then(() => setCopied(true), () => setCopied(false)); }}>{copied ? 'Copied' : 'Copy token'}</button><button class="button button--quiet" type="button" onClick={() => setSecret(null)}>Hide token</button></div></div>}
      <form onSubmit={(e) => { e.preventDefault(); void run(async () => {
        setSecret(null);
        const created = await requestJson('/api/v1/auth/server-tokens', (value) => expectString(expectRecord(value, 'new token'), 'token', 'new token'), { csrfToken, method: 'POST', body: { name: name.trim(), servers: selected, permissions, expires_in_days: days } });
        setSecret(created); setName(''); await refresh();
      }); }}>
        <div class="server-token-settings__fields"><label>Token name<input required maxLength={80} value={name} onInput={(e) => setName(e.currentTarget.value)} placeholder="Plugin deployment" /></label><label>Expires after<select value={days === null ? 'never' : days} onChange={(e) => setDays(e.currentTarget.value === 'never' ? null : Number(e.currentTarget.value))}>{EXPIRY_DAYS.map((d) => <option key={d} value={d}>{expiryLabel(d)}</option>)}<option value="never">Never — revoke manually</option></select></label></div>
        <fieldset disabled={busy}><legend>Servers</legend>{servers.length === 0 && <p>No servers available.</p>}{servers.map((s) => <label key={s.id}><input type="checkbox" checked={selected.includes(s.id)} onChange={() => setSelected(toggle(selected, s.id))} /><span>{s.name}<small>{s.id}</small></span></label>)}</fieldset>
        <fieldset disabled={busy}><legend>Permissions</legend>{data.permissions.map((p) => <label key={p}><input type="checkbox" checked={permissions.includes(p)} onChange={() => setPermissions(toggle(permissions, p))} /><span>{p}</span></label>)}</fieldset>
        <button class="button button--primary" disabled={busy || name.trim() === '' || selected.length === 0 || permissions.length === 0}>{busy ? 'Working…' : 'Create token'}</button>
      </form>
      <div class="server-token-settings__list">{data.tokens.length === 0 && <p>No tokens yet.</p>}{data.tokens.map((t) => { const status = tokenStatus(t, Date.now()); return <article key={t.id}>
        <div><strong>{t.name}</strong><small class={status.usable ? 'server-token-settings__ok' : 'server-token-settings__bad'}>{status.label}</small><small>Servers: {t.servers.map(serverName).join(', ')}</small><small>Permissions: {t.permissions.join(', ')}</small><small>{t.createdAt !== null && `Created ${when(t.createdAt)} · `}{t.lastUsedAt === null ? 'Never used' : `Last used ${when(t.lastUsedAt)}`}</small></div>
        {!t.revoked && <div class="server-token-settings__actions">
          {confirmRotate === t.id ? <><small>The old token will stop working immediately.</small><button class="button button--primary" type="button" disabled={busy} onClick={() => void run(async () => {
            const next = await requestJson(`/api/v1/auth/server-tokens/${encodeURIComponent(t.id)}/rotate`, (v) => expectString(expectRecord(v, 'rotated token'), 'token', 'rotated token'), { csrfToken, method: 'POST', body: {} });
            setSecret(next); setConfirmRotate(null); await refresh();
          })}>Confirm rotation</button><button class="button button--quiet" type="button" disabled={busy} onClick={() => setConfirmRotate(null)}>Cancel</button></>
          : confirmRevoke === t.id ? <><button class="button button--danger" type="button" disabled={busy} onClick={() => void run(async () => { await requestJson(`/api/v1/auth/server-tokens/${encodeURIComponent(t.id)}`, (v) => expectRecord(v, 'revocation'), { csrfToken, method: 'DELETE', body: {} }); setConfirmRevoke(null); await refresh(); })}>Confirm revoke</button><button class="button button--quiet" type="button" disabled={busy} onClick={() => setConfirmRevoke(null)}>Cancel</button></>
          : <><button class="button button--quiet" type="button" disabled={busy || !status.usable} onClick={() => { setConfirmRevoke(null); setConfirmRotate(t.id); }}>Rotate &amp; show new token</button><button class="button button--quiet" type="button" disabled={busy} onClick={() => { setConfirmRotate(null); setConfirmRevoke(t.id); }}>Revoke</button></>}
        </div>}
      </article>; })}</div>
      <button class="button button--quiet" disabled={busy} onClick={() => void run(refresh)}>Refresh tokens</button>
    </>}
  </section>;
}
