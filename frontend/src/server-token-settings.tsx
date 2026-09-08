import { useState } from 'preact/hooks';
import { expectArray, expectNumber, expectRecord, expectString, requestJson } from './api';
import type { ManagedServer } from './control-api';
import { InlineError } from './dashboard-ui';
import { Icon } from './icons';
import './server-token-settings.css';

interface Token { id: string; name: string; servers: string[]; permissions: string[]; expiresAt: number; revoked: boolean }
function catalog(value: unknown): { tokens: Token[]; permissions: string[] } {
  const data = expectRecord(value, 'API tokens');
  return {
    permissions: expectArray(data, 'permissions', 'API tokens', 64).map((p) => expectString({ value: p }, 'value', 'permission')),
    tokens: expectArray(data, 'tokens', 'API tokens', 256).map((value) => {
      const t = expectRecord(value, 'token');
      return { id: expectString(t, 'id', 'token'), name: expectString(t, 'name', 'token'), servers: expectArray(t, 'servers', 'token', 64).map((s) => expectString({ value: s }, 'value', 'server')), permissions: expectArray(t, 'permissions', 'token', 64).map((p) => expectString({ value: p }, 'value', 'permission')), expiresAt: expectNumber(t, 'expires_at', 'token'), revoked: t.revoked_at !== null };
    }),
  };
}

export function ServerTokenSettings({ csrfToken, servers }: { csrfToken: string; servers: ManagedServer[] }) {
  const [data, setData] = useState<ReturnType<typeof catalog> | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [days, setDays] = useState(30);
  const [selected, setSelected] = useState<string[]>([]);
  const [permissions, setPermissions] = useState<string[]>(['view']);
  const [secret, setSecret] = useState<string | null>(null);
  const [confirmRevoke, setConfirmRevoke] = useState<string | null>(null);
  async function refresh() { setData(await requestJson('/api/v1/auth/server-tokens', catalog, { csrfToken })); }
  async function run(operation: () => Promise<void>) {
    if (busy) return;
    setBusy(true); setError(null);
    try { await operation(); } catch (e) { setError(e instanceof Error ? e.message : 'The request failed. Refresh the token list before trying again.'); }
    finally { setBusy(false); }
  }
  function toggle(list: string[], id: string): string[] { return list.includes(id) ? list.filter((x) => x !== id) : [...list, id]; }
  return <section class="settings-card server-token-settings">
    <div class="settings-card__head"><div><Icon name="servers" /><span><h2>Server API tokens</h2><p>Give a tool access to selected servers without sharing your login.</p></span></div></div>
    <InlineError message={error} />
    {data === null ? <button class="button button--quiet" disabled={busy} onClick={() => void run(refresh)}>{busy ? 'Loading…' : 'Manage tokens'}</button> : <>
      <p class="server-token-settings__warning">Use HTTPS or an SSH tunnel. File writes and console commands are powerful: only grant them to trusted tools. Tokens cannot administer the host or create other tokens.</p>
      {secret !== null && <div class="server-token-settings__secret" role="status"><strong>Save this token now. It is shown only once.</strong><input aria-label="New API token" readOnly value={secret} autoComplete="off" spellcheck={false} onFocus={(e) => e.currentTarget.select()} /><button class="button button--quiet" onClick={() => setSecret(null)}>I saved it — hide token</button></div>}
      <form onSubmit={(e) => { e.preventDefault(); void run(async () => {
        setSecret(null);
        const created = await requestJson('/api/v1/auth/server-tokens', (value) => expectString(expectRecord(value, 'new token'), 'token', 'new token'), { csrfToken, method: 'POST', body: { name: name.trim(), servers: selected, permissions, expires_in_days: days } });
        setSecret(created); setName(''); await refresh();
      }); }}>
        <div class="server-token-settings__fields"><label>Token name<input required maxLength={80} value={name} onInput={(e) => setName(e.currentTarget.value)} placeholder="Plugin deployment" /></label><label>Expires after<select value={days} onChange={(e) => setDays(Number(e.currentTarget.value))}>{[1, 7, 30, 90].map((d) => <option key={d} value={d}>{d} days</option>)}</select></label></div>
        <fieldset disabled={busy}><legend>Servers</legend>{servers.length === 0 && <p>No servers available.</p>}{servers.map((s) => <label key={s.id}><input type="checkbox" checked={selected.includes(s.id)} onChange={() => setSelected(toggle(selected, s.id))} /><span>{s.name}<small>{s.id}</small></span></label>)}</fieldset>
        <fieldset disabled={busy}><legend>Permissions</legend>{data.permissions.map((p) => <label key={p}><input type="checkbox" checked={permissions.includes(p)} onChange={() => setPermissions(toggle(permissions, p))} /><span>{p}</span></label>)}</fieldset>
        <button class="button button--primary" disabled={busy || name.trim() === '' || selected.length === 0 || permissions.length === 0}>{busy ? 'Working…' : 'Create token'}</button>
      </form>
      <div class="server-token-settings__list">{data.tokens.map((t) => <article key={t.id}><div><strong>{t.name}</strong><small>{t.revoked ? 'Revoked' : t.expiresAt <= Date.now() ? 'Expired' : `Expires ${new Date(t.expiresAt).toLocaleDateString()}`}</small><small>{t.permissions.join(', ')} · {t.servers.length} server(s)</small></div>{!t.revoked && (confirmRevoke === t.id ? <div><button class="button button--danger" disabled={busy} onClick={() => void run(async () => { await requestJson(`/api/v1/auth/server-tokens/${encodeURIComponent(t.id)}`, (v) => expectRecord(v, 'revocation'), { csrfToken, method: 'DELETE', body: {} }); setConfirmRevoke(null); await refresh(); })}>Confirm revoke</button><button class="button button--quiet" disabled={busy} onClick={() => setConfirmRevoke(null)}>Cancel</button></div> : <button class="button button--quiet" disabled={busy} onClick={() => setConfirmRevoke(t.id)}>Revoke</button>)}</article>)}</div>
      <button class="button button--quiet" disabled={busy} onClick={() => void run(refresh)}>Refresh tokens</button>
    </>}
  </section>;
}
