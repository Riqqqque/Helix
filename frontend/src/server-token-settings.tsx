import { CopyButton } from './copy-button';
import { useState } from 'preact/hooks';
import { ApiError, expectArray, expectNumber, expectRecord, expectString, requestJson } from './api';
import type { ManagedServer } from './control-api';
import { InlineError } from './dashboard-ui';
import { Icon } from './icons';
import './server-token-settings.css';

interface Token { id: string; name: string; servers: string[]; permissions: string[]; createdAt: number | null; expiresAt: number | null; lastUsedAt: number | null; revoked: boolean; authorized: boolean; viewable: boolean }
const EXPIRY_DAYS = [1, 7, 30, 90, 180, 365];
/** Grants every server permission, including ones added later. Never host controls. */
export const ALL_PERMISSIONS = 'all';
const PERMISSION_LABELS: Record<string, { title: string; detail: string }> = {
  view: { title: 'View', detail: 'Status, details, marketplace search' },
  logs: { title: 'Logs', detail: 'Read console output' },
  'files.read': { title: 'Read files', detail: 'List and download' },
  'files.write': { title: 'Write files', detail: 'Upload plugins, edit, install' },
  start: { title: 'Start', detail: 'Start the server' },
  stop: { title: 'Stop', detail: 'Stop cleanly' },
  restart: { title: 'Restart', detail: 'Restart the server' },
  kill: { title: 'Kill', detail: 'Force stop' },
  console: { title: 'Console', detail: 'Run commands' },
  settings: { title: 'Settings', detail: 'Properties, memory, ports' },
  update: { title: 'Update', detail: 'Server software and packs' },
  'backups.read': { title: 'Read backups', detail: 'List and download' },
  'backups.write': { title: 'Manage backups', detail: 'Create, restore, delete' },
  network: { title: 'Network', detail: 'Open or close to the internet' },
  remove: { title: 'Remove server', detail: 'Move to Removed servers' },
};
export function permissionLabel(permission: string): string {
  if (permission === ALL_PERMISSIONS) return 'Full access';
  return PERMISSION_LABELS[permission]?.title ?? permission;
}
function optionalTime(t: Record<string, unknown>, key: string): number | null { return t[key] === null || t[key] === undefined ? null : expectNumber(t, key, 'token'); }
function when(ms: number): string { return new Date(ms).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }); }
function expiryLabel(days: number): string { return days === 365 ? '1 year' : days === 1 ? '1 day' : `${days} days`; }
export function catalog(value: unknown): { tokens: Token[]; permissions: string[] } {
  const data = expectRecord(value, 'API tokens');
  return {
    permissions: expectArray(data, 'permissions', 'API tokens', 64).map((p) => expectString({ value: p }, 'value', 'permission')),
    tokens: expectArray(data, 'tokens', 'API tokens', 256).map((value) => {
      const t = expectRecord(value, 'token');
      return { id: expectString(t, 'id', 'token'), name: expectString(t, 'name', 'token'), servers: expectArray(t, 'servers', 'token', 64).map((s) => expectString({ value: s }, 'value', 'server')), permissions: expectArray(t, 'permissions', 'token', 64).map((p) => expectString({ value: p }, 'value', 'permission')), createdAt: optionalTime(t, 'created_at'), expiresAt: optionalTime(t, 'expires_at'), lastUsedAt: optionalTime(t, 'last_used_at'), revoked: t.revoked_at !== null && t.revoked_at !== undefined, authorized: t.authorized !== false, viewable: t.viewable === true };
    }),
  };
}
export function tokenStatus(t: Token, now: number): { label: string; usable: boolean } {
  if (t.revoked) return { label: 'Revoked', usable: false };
  if (t.expiresAt !== null && t.expiresAt <= now) return { label: 'Expired', usable: false };
  if (!t.authorized) return { label: 'Invalid — your account or password changed', usable: false };
  return { label: t.expiresAt === null ? 'Active · never expires' : `Active · expires ${when(t.expiresAt)}`, usable: true };
}
/** Individual permissions shown as checkboxes; Full access is offered separately. */
export function individualPermissions(permissions: string[]): string[] {
  return permissions.filter((p) => p !== ALL_PERMISSIONS);
}
function secretFrom(value: unknown, context: string): { token: string; viewable: boolean } {
  const record = expectRecord(value, context);
  return { token: expectString(record, 'token', context), viewable: record.viewable === true };
}

export function ServerTokenSettings({ csrfToken, servers }: { csrfToken: string; servers: ManagedServer[] }) {
  const [data, setData] = useState<ReturnType<typeof catalog> | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [days, setDays] = useState<number | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [permissions, setPermissions] = useState<string[]>([ALL_PERMISSIONS]);
  const [secret, setSecretValue] = useState<{ token: string; title: string; note: string } | null>(null);
  const [confirmRevoke, setConfirmRevoke] = useState<string | null>(null);
  const [confirmRotate, setConfirmRotate] = useState<string | null>(null);
  const [viewing, setViewing] = useState<string | null>(null);
  const [password, setPassword] = useState('');
  const [passwordError, setPasswordError] = useState<string | null>(null);
  function setSecret(value: { token: string; title: string; note: string } | null) { setSecretValue(value); }
  async function refresh() { setData(await requestJson('/api/v1/auth/server-tokens', catalog, { csrfToken })); }
  async function run(operation: () => Promise<void>) {
    if (busy) return;
    setBusy(true); setError(null);
    try { await operation(); } catch (e) { setError(e instanceof Error ? e.message : 'The request failed. Refresh the token list before trying again.'); }
    finally { setBusy(false); }
  }
  function toggle(list: string[], id: string): string[] { return list.includes(id) ? list.filter((x) => x !== id) : [...list, id]; }
  function serverName(id: string): string { return servers.find((s) => s.id === id)?.name ?? id; }
  function closeRowActions() { setConfirmRevoke(null); setConfirmRotate(null); setViewing(null); setPassword(''); setPasswordError(null); }
  const issuedNote = (viewable: boolean) => viewable ? 'You can view it again any time with View token.' : 'Copy it now. Helix could not keep a viewable copy, so use Rotate if you lose it.';
  const fullAccess = permissions.includes(ALL_PERMISSIONS);
  const individual = data === null ? [] : individualPermissions(data.permissions);
  const allServersSelected = servers.length > 0 && servers.every((s) => selected.includes(s.id));

  async function reveal(token: Token) {
    if (busy) return;
    setBusy(true); setPasswordError(null); setError(null);
    try {
      const shown = await requestJson(`/api/v1/auth/server-tokens/${encodeURIComponent(token.id)}/reveal`, (v) => secretFrom(v, 'token'), { csrfToken, method: 'POST', body: { current_password: password } });
      setSecret({ token: shown.token, title: token.name, note: 'Keep it private. Anyone with this value can use the permissions listed on the token.' });
      closeRowActions();
    } catch (e) {
      if (e instanceof ApiError && e.code === 'current_password_rejected') setPasswordError('That password is not correct.');
      else if (e instanceof ApiError && e.status === 404) setPasswordError('This token has no viewable copy. Use Rotate to get a new value you can view later.');
      else setPasswordError(e instanceof Error ? e.message : 'Could not show the token.');
    } finally {
      setBusy(false); setPassword('');
    }
  }

  return <section class="settings-card server-token-settings">
    <div class="settings-card__head"><div><Icon name="servers" /><span><h2>Server API tokens</h2><p>Give a tool or AI access to selected servers without sharing your login.</p></span></div></div>
    {error !== null && <div class="server-token-settings__error"><InlineError message={error} /></div>}
    {data === null ? <div class="server-token-settings__intro">
      <p>Create, view, rotate, and revoke tokens for scripts, plugins, and AI tools.</p>
      <button class="button button--quiet" type="button" disabled={busy} onClick={() => void run(refresh)}>{busy ? 'Loading…' : 'Manage tokens'}</button>
    </div> : <>
      {secret !== null && <div class="server-token-settings__secret" role="status">
        <strong>{secret.title}</strong>
        <small>{secret.note}</small>
        <div class="server-token-settings__secret-row">
          <input aria-label="API token" readOnly value={secret.token} autoComplete="off" spellcheck={false} onFocus={(e) => e.currentTarget.select()} />
          <CopyButton key={secret.token} text={secret.token} class="button button--primary" />
          <button class="button button--quiet" type="button" onClick={() => setSecret(null)}>Hide</button>
        </div>
      </div>}
      <form class="server-token-settings__form" onSubmit={(e) => { e.preventDefault(); void run(async () => {
        setSecret(null);
        const created = await requestJson('/api/v1/auth/server-tokens', (value) => secretFrom(value, 'new token'), { csrfToken, method: 'POST', body: { name: name.trim(), servers: selected, permissions, expires_in_days: days } });
        setSecret({ token: created.token, title: `New token · ${name.trim()}`, note: issuedNote(created.viewable) }); setName(''); await refresh();
      }); }}>
        <h3>New token</h3>
        <div class="server-token-settings__fields">
          <label><span>Token name</span><input required maxLength={80} value={name} onInput={(e) => setName(e.currentTarget.value)} placeholder="Plugin deployment" /></label>
          <label><span>Expires after</span><select value={days === null ? 'never' : days} onChange={(e) => setDays(e.currentTarget.value === 'never' ? null : Number(e.currentTarget.value))}>{EXPIRY_DAYS.map((d) => <option key={d} value={d}>{expiryLabel(d)}</option>)}<option value="never">Never (until revoked)</option></select></label>
        </div>
        <fieldset disabled={busy}>
          <div class="server-token-settings__legend-row"><legend>Servers</legend>{servers.length > 0 && <button class="button button--quiet button--small" type="button" onClick={() => setSelected(allServersSelected ? [] : servers.map((s) => s.id))}>{allServersSelected ? 'Clear' : 'Select all'}</button>}</div>
          {servers.length === 0 && <p class="server-token-settings__empty">No servers available.</p>}
          <div class="server-token-settings__choices server-token-settings__choices--servers">{servers.map((s) => <label key={s.id}><input type="checkbox" checked={selected.includes(s.id)} onChange={() => setSelected(toggle(selected, s.id))} /><span><strong>{s.name}</strong><small>{s.id}</small></span></label>)}</div>
        </fieldset>
        <fieldset disabled={busy}>
          <div class="server-token-settings__legend-row"><legend>Permissions</legend>{!fullAccess && <button class="button button--quiet button--small" type="button" onClick={() => setPermissions(permissions.length === individual.length ? [] : [...individual])}>{permissions.length === individual.length ? 'Clear' : 'Select all'}</button>}</div>
          {data.permissions.includes(ALL_PERMISSIONS) && <label class="server-token-settings__full">
            <input type="checkbox" checked={fullAccess} onChange={() => setPermissions(fullAccess ? ['view'] : [ALL_PERMISSIONS])} />
            <span><strong>Full access</strong><small>Everything on the selected servers — files, console, plugins, restarts, backups, settings, updates, network, and removal — including permissions added in future Helix versions. Never the host itself.</small></span>
          </label>}
          {!fullAccess && <div class="server-token-settings__choices">{individual.map((p) => <label key={p}><input type="checkbox" checked={permissions.includes(p)} onChange={() => setPermissions(toggle(permissions, p))} /><span><strong>{permissionLabel(p)}</strong>{PERMISSION_LABELS[p] && <small class="server-token-settings__hint">{PERMISSION_LABELS[p].detail}</small>}</span></label>)}</div>}
        </fieldset>
        <div class="server-token-settings__form-actions">
          <span>Use HTTPS or an SSH tunnel. Tokens reach only the servers you pick and can't administer the host.</span>
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
              <dt>Permissions</dt><dd>{t.permissions.map(permissionLabel).join(', ')}</dd>
              <dt>Created</dt><dd>{t.createdAt === null ? '—' : when(t.createdAt)}</dd>
              <dt>Last used</dt><dd>{t.lastUsedAt === null ? 'Never' : when(t.lastUsedAt)}</dd>
            </dl>
          </div>
          {!t.revoked && <div class="server-token-settings__actions">
            {viewing === t.id ? <form class="server-token-settings__reveal" onSubmit={(e) => { e.preventDefault(); void reveal(t); }}>
              <small>Enter your dashboard password to show this token.</small>
              <input type="password" aria-label="Dashboard password" autoComplete="current-password" required value={password} disabled={busy} onInput={(e) => setPassword(e.currentTarget.value)} />
              {passwordError !== null && <small class="server-token-settings__reveal-error" role="alert">{passwordError}</small>}
              <div><button class="button button--primary" type="submit" disabled={busy || password === ''}>{busy ? 'Checking…' : 'Show token'}</button><button class="button button--quiet" type="button" disabled={busy} onClick={closeRowActions}>Cancel</button></div>
            </form>
            : confirmRotate === t.id ? <><small>The current value stops working immediately.</small><button class="button button--primary" type="button" disabled={busy} onClick={() => void run(async () => {
              const next = await requestJson(`/api/v1/auth/server-tokens/${encodeURIComponent(t.id)}/rotate`, (v) => secretFrom(v, 'rotated token'), { csrfToken, method: 'POST', body: {} });
              setSecret({ token: next.token, title: `New value · ${t.name}`, note: issuedNote(next.viewable) }); setConfirmRotate(null); await refresh();
            })}>Confirm rotate</button><button class="button button--quiet" type="button" disabled={busy} onClick={() => setConfirmRotate(null)}>Cancel</button></>
            : confirmRevoke === t.id ? <><small>Tools using this token lose access.</small><button class="button button--danger" type="button" disabled={busy} onClick={() => void run(async () => { await requestJson(`/api/v1/auth/server-tokens/${encodeURIComponent(t.id)}`, (v) => expectRecord(v, 'revocation'), { csrfToken, method: 'DELETE', body: {} }); setConfirmRevoke(null); await refresh(); })}>Confirm revoke</button><button class="button button--quiet" type="button" disabled={busy} onClick={() => setConfirmRevoke(null)}>Cancel</button></>
            : <>
              {t.viewable
                ? <button class="button button--primary" type="button" disabled={busy || !status.usable} onClick={() => { closeRowActions(); setViewing(t.id); }}><Icon name="eye" size={13} />View token</button>
                : status.usable && <small>Made before tokens could be viewed. Rotate once to get a value you can view any time.</small>}
              <button class="button button--quiet" type="button" disabled={busy || !status.usable} title="Issue a new value for this token" onClick={() => { closeRowActions(); setConfirmRotate(t.id); }}>Rotate</button>
              <button class="button button--quiet" type="button" disabled={busy} onClick={() => { closeRowActions(); setConfirmRevoke(t.id); }}>Revoke</button>
            </>}
          </div>}
        </article>; })}
      </div>
      <div class="settings-card__foot"><span>Token values are encrypted at rest; viewing one needs your password</span><button class="button button--quiet" type="button" disabled={busy} onClick={() => void run(refresh)}>Refresh</button></div>
    </>}
  </section>;
}
