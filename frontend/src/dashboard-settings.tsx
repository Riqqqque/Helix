import { useEffect, useState } from 'preact/hooks';
import { updateAccount } from './api';
import type { DashboardResource } from './dashboard-model';
import {
  addableDashboardSections,
  hideDashboardPage,
  moveVisibleNavigationItem,
  showDashboardPage,
} from './dashboard-page-catalog';
import {
  defaultHiddenPages,
  primaryDashboardSections,
  refreshIntervalOptions,
  visibleDashboardSections,
  type DashboardColors,
  type PrimaryDashboardSectionId,
  type RefreshIntervalMs,
} from './dashboard-preferences';
import { Icon, type IconName } from './icons';
import { InfoTip } from './info-tip';
import {
  cancelHostReboot,
  deleteRecurringHostReboot,
  getHostRebootPreflight,
  scheduleHostReboot,
  setRecurringHostReboot,
  setHelixStartOnBoot,
  type HostIntegration,
  type HostRebootPreflight,
  type RebootWeekday,
  type ScheduledReboot,
  type ScheduledRebootStatus,
} from './host-api';
import { InlineError } from './dashboard-ui';
import { Dialog } from './modal';
import { formatTimestamp } from './format';
import type { ThemePreference } from './theme';
import type { AuthenticatedUser } from './types';
import type { ManagedServer, TrashedNativeServer, TrashedNativeServerCatalog } from './control-api';
import {
  getTrashedNativeServers,
  restoreTrashedNativeServer,
  setNativeStartOnBoot,
  serverStatusLabel,
  trashNativeServer,
} from './control-api';
import { purgeTrashedNativeServer } from './native-server-trash-api';
import {
  CURSEFORGE_CONSOLE_URL,
  clearCurseforgeApiKey,
  getCurseforgeKeyStatus,
  normalizeCurseforgeApiKey,
  setCurseforgeApiKey,
} from './curseforge-key-api';
import './catalogs-settings.css';
import {
  readForgottenImportedServers,
  rememberImportedServer,
  readHiddenImportedServers,
} from './imported-server-visibility';
import {
  clearDismissals,
  DISMISSALS_CHANGED_EVENT,
  listDismissedIds,
} from './dismissals';
import { GameMark, gameMarkForSoftware } from './game-marks';
import { serverDetailHash } from './server-hash';
import {
  START_WITH_HOST_DETAIL,
  START_WITH_HOST_TITLE,
} from './start-with-host';

const navigationLabels: Record<PrimaryDashboardSectionId, { label: string; icon: IconName }> = {
  overview: { label: 'Overview', icon: 'overview' },
  home: { label: 'Home', icon: 'home' },
  storage: { label: 'Storage', icon: 'storage' },
  network: { label: 'Network', icon: 'network' },
  host: { label: 'Host', icon: 'host' },
  security: { label: 'Security', icon: 'security' },
  terminal: { label: 'Terminal', icon: 'terminal' },
  servers: { label: 'Servers', icon: 'servers' },
  hooks: { label: 'Hooks', icon: 'hooks' },
  strands: { label: 'Strands', icon: 'strands' },
  globe: { label: 'Globe', icon: 'globe' },
};

const themeLabels: Record<ThemePreference, string> = {
  system: 'System',
  midnight: 'Midnight',
  oled: 'OLED',
  light: 'Light',
};

function intervalLabel(value: RefreshIntervalMs): string {
  return value < 10_000 ? `${value / 1_000} ${value === 1_000 ? 'second' : 'seconds'}` : `${value / 1_000} seconds`;
}

function accountValidation(
  loginName: string,
  displayName: string,
  currentPassword: string,
  newPassword: string,
  confirmation: string,
): string | null {
  if (!/^[a-z0-9](?:[a-z0-9._-]{1,62}[a-z0-9])$/u.test(loginName)) {
    return 'Use 3–64 lowercase letters, numbers, dots, underscores, or hyphens for the login name.';
  }
  const canonicalDisplayName = displayName.normalize('NFC');
  if (
    canonicalDisplayName.trim() !== canonicalDisplayName ||
    Array.from(canonicalDisplayName).length < 1 ||
    Array.from(canonicalDisplayName).length > 128 ||
    new TextEncoder().encode(canonicalDisplayName).length > 512 ||
    Array.from(canonicalDisplayName).some((character) => /\p{Cc}/u.test(character))
  ) return 'Use a display name from 1–128 characters with no leading, trailing, or control characters.';
  if (currentPassword.length === 0) return 'Enter your current password to authorize the change.';
  if (newPassword.length === 0) return confirmation.length === 0 ? null : 'Enter the new password before confirming it.';
  if (newPassword !== confirmation) return 'The new password confirmation does not match.';
  const canonicalPassword = newPassword.normalize('NFC');
  const points = Array.from(canonicalPassword).length;
  if (points < 13 || points > 256 || new TextEncoder().encode(canonicalPassword).length > 1_024) {
    return 'Use 13–256 characters for the new password.';
  }
  return null;
}

function AccountSettings({
  user,
  csrfToken,
  onAccountUpdated,
}: {
  user: AuthenticatedUser;
  csrfToken: string;
  onAccountUpdated: () => void;
}) {
  const [loginName, setLoginName] = useState(user.loginName);
  const [displayName, setDisplayName] = useState(user.displayName);
  const [currentPassword, setCurrentPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const changed = loginName !== user.loginName || displayName !== user.displayName || newPassword.length > 0;

  const submit = async (event: Event): Promise<void> => {
    event.preventDefault();
    if (busy) return;
    const validation = accountValidation(loginName, displayName, currentPassword, newPassword, confirmation);
    if (validation !== null) {
      setError(validation);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await updateAccount({
        currentPassword,
        loginName,
        displayName: displayName.normalize('NFC'),
        ...(newPassword.length === 0 ? {} : { newPassword: newPassword.normalize('NFC') }),
      }, csrfToken);
      setCurrentPassword('');
      setNewPassword('');
      setConfirmation('');
      onAccountUpdated();
    } catch (requestError) {
      setCurrentPassword('');
      setError(requestError instanceof Error ? requestError.message : 'Helix could not update the account.');
    } finally {
      setBusy(false);
    }
  };

  return (
    <section class="settings-card settings-card--account">
      <div class="settings-card__head"><div><Icon name="user" /><span><h2>Owner account</h2><p>Change the name used to sign in or replace the owner password.</p></span></div><InfoTip text="Helix has one founding owner account. Saving revokes every active session, including this one, so stolen sessions cannot survive a credential change." /></div>
      <form class="account-settings-form" onSubmit={(event) => void submit(event)}>
        <div class="settings-field-row">
          <label><span>Login name</span><input value={loginName} minLength={3} maxLength={64} autoComplete="username" autocapitalize="none" spellcheck={false} onInput={(event) => setLoginName(event.currentTarget.value.toLowerCase())} /></label>
          <label><span>Display name</span><input value={displayName} maxLength={512} autoComplete="name" onInput={(event) => setDisplayName(event.currentTarget.value)} /></label>
        </div>
        <div class="settings-field-row">
          <label><span>New password <small>Optional</small></span><input type="password" value={newPassword} minLength={13} maxLength={1_024} autoComplete="new-password" onInput={(event) => setNewPassword(event.currentTarget.value)} /></label>
          <label><span>Confirm new password</span><input type="password" value={confirmation} minLength={newPassword.length === 0 ? undefined : 13} maxLength={1_024} autoComplete="new-password" onInput={(event) => setConfirmation(event.currentTarget.value)} /></label>
        </div>
        <label class="account-current-password"><span>Current password</span><input type="password" value={currentPassword} maxLength={1_024} autoComplete="current-password" onInput={(event) => setCurrentPassword(event.currentTarget.value)} /><small>Required for every account change. Helix never stores it in browser preferences.</small></label>
        {error !== null && <div class="settings-form-error" role="alert"><Icon name="warning" size={15} />{error}</div>}
        <div class="settings-form-actions"><span>Email is not part of Helix’s local owner authentication.</span><button class="button button--primary" type="submit" disabled={busy || !changed || currentPassword.length === 0}>{busy ? 'Saving…' : 'Save account changes'}</button></div>
      </form>
    </section>
  );
}

function CatalogsSettings({
  user,
  csrfToken,
}: {
  user: AuthenticatedUser;
  csrfToken: string;
}) {
  const canView = user.capabilities.includes('games.view');
  const canManage = user.capabilities.includes('games.manage');
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [key, setKey] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!canView) return;
    const controller = new AbortController();
    void getCurseforgeKeyStatus(csrfToken, controller.signal)
      .then((status) => {
        setConfigured(status.configured);
        setError(null);
        setNotice(null);
      })
      .catch((requestError: unknown) => {
        if (controller.signal.aborted) return;
        setConfigured(null);
        setError(requestError instanceof Error ? requestError.message : 'Helix could not read the CurseForge catalog setting.');
      });
    return () => controller.abort();
  }, [canView, csrfToken]);

  if (!canView && !canManage) return null;

  const save = async (event: Event): Promise<void> => {
    event.preventDefault();
    if (!canManage || busy) return;
    const trimmed = normalizeCurseforgeApiKey(key);
    if (trimmed.length < 24 || trimmed.length > 256) {
      setError('CurseForge API keys are 24–256 characters.');
      return;
    }
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const status = await setCurseforgeApiKey(trimmed, csrfToken);
      setKey('');
      setConfigured(status.configured);
      if (status.probe === 'cdn_blocked') {
        setNotice("Saved. CurseForge blocked this server's public IP. The key is fine. Search works once this host exits on a normal ISP address instead of a VPS/VPN.");
      } else if (status.probe === 'unreachable') {
        setNotice('Saved. Helix could not reach CurseForge to verify the key.');
      }
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : 'Helix could not save the CurseForge API key.');
    } finally {
      setBusy(false);
    }
  };

  const remove = async (): Promise<void> => {
    if (!canManage || busy || configured !== true) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const status = await clearCurseforgeApiKey(csrfToken);
      setKey('');
      setConfigured(status.configured);
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : 'Helix could not remove the CurseForge API key.');
    } finally {
      setBusy(false);
    }
  };

  return (
    <section class="settings-card">
      <div class="settings-card__head"><div><Icon name="search" /><span><h2>Catalogs</h2><p>CurseForge downloads need your own API key. Modrinth stays public.</p></span></div><InfoTip text="Helix stores the key only on this host, never in the browser or the dashboard database, and never shows it again. If CurseForge's network blocks this server, the key is still kept." /></div>
      <div class="host-boot-control">
        <div>
          <span>CurseForge API key</span>
          <strong>{error !== null && configured === null ? 'Unavailable' : configured === true ? 'Saved on this host' : configured === false ? 'Not saved' : canView ? 'Checking…' : 'Unknown'}</strong>
          <small>Get a key from the CurseForge console, then marketplace search and “Start with a modpack” can download from api.curseforge.com.</small>
        </div>
        <a class="button button--quiet" href={CURSEFORGE_CONSOLE_URL} target="_blank" rel="noreferrer">Open CurseForge console <Icon name="external" size={14} /></a>
      </div>
      {canManage && (
        <form class="account-settings-form" onSubmit={(event) => void save(event)}>
          <label class="account-current-password catalogs-key-field">
            <span>API key</span>
            <input
              type="password"
              value={key}
              maxLength={512}
              autoComplete="off"
              spellcheck={false}
              autocapitalize="none"
              placeholder={configured === true ? 'Paste a replacement key' : 'Paste the key from console.curseforge.com'}
              onInput={(event) => setKey(event.currentTarget.value)}
            />
            <small>Paste from the console. Hidden characters, quotes, and docker $$ wrapping are stripped. Helix keeps the key in a private file on this host and never shows it again.</small>
          </label>
          {notice !== null && <div class="settings-form-error" role="status"><Icon name="warning" size={15} />{notice}</div>}
          {error !== null && <div class="settings-form-error" role="alert"><Icon name="warning" size={15} />{error}</div>}
          <div class="settings-form-actions">
            <span>Removing the key stops new CurseForge downloads until another is saved.</span>
            <span class="catalogs-key-actions">
              {configured === true && <button class="button button--quiet" type="button" disabled={busy} onClick={() => void remove()}>{busy ? 'Working…' : 'Remove'}</button>}
              <button class="button button--primary" type="submit" disabled={busy || key.trim().length === 0}>{busy ? 'Working…' : 'Save key'}</button>
            </span>
          </div>
        </form>
      )}
    </section>
  );
}

export function validateHostReboot(
  hostname: string,
  confirmation: string,
  acknowledged: boolean,
  delaySeconds: number,
  preflight: HostRebootPreflight | null,
): string | null {
  if (preflight === null) return 'Wait for Helix to finish the reboot safety check.';
  if (!preflight.canSchedule) return 'Resolve every preflight blocker before scheduling a reboot.';
  if (confirmation !== hostname) return `Type ${hostname} exactly to confirm this host.`;
  if (!acknowledged) return 'Acknowledge that every service and player connection will be interrupted.';
  if (!Number.isInteger(delaySeconds) || delaySeconds < 10 || delaySeconds > 300) return 'Choose a delay from 10 to 300 seconds.';
  return null;
}

type ActiveReboot = ScheduledReboot | Extract<ScheduledRebootStatus, { state: 'scheduled' | 'executing' }>;

function HostRebootDialog({
  integration,
  csrfToken,
  onClose,
  onChanged,
}: {
  integration: HostIntegration;
  csrfToken: string;
  onClose: () => void;
  onChanged: () => Promise<void>;
}) {
  const initialScheduled = integration.scheduledReboot.state === 'none' ? null : integration.scheduledReboot;
  const [scheduled, setScheduled] = useState<ActiveReboot | null>(initialScheduled);
  const [preflight, setPreflight] = useState<HostRebootPreflight | null>(initialScheduled === null ? null : integration.rebootPreflight);
  const [loading, setLoading] = useState(initialScheduled === null);
  const [confirmation, setConfirmation] = useState('');
  const [acknowledged, setAcknowledged] = useState(false);
  const [delaySeconds, setDelaySeconds] = useState(30);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());

  const refreshPreflight = async (): Promise<void> => {
    setLoading(true);
    setError(null);
    try {
      setPreflight(await getHostRebootPreflight(csrfToken));
    } catch (requestError) {
      setPreflight(null);
      setError(requestError instanceof Error ? requestError.message : 'Helix could not run the reboot safety check.');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (initialScheduled === null) void refreshPreflight();
  }, []);
  useEffect(() => {
    setScheduled(integration.scheduledReboot.state === 'none' ? null : integration.scheduledReboot);
  }, [integration.scheduledReboot]);
  useEffect(() => {
    if (scheduled === null) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [scheduled]);

  const submit = async (): Promise<void> => {
    const validation = validateHostReboot(integration.hostname, confirmation, acknowledged, delaySeconds, preflight);
    if (validation !== null) {
      setError(validation);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const result = await scheduleHostReboot(integration.hostname, delaySeconds, csrfToken);
      setScheduled(result);
      await onChanged();
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : 'Helix could not schedule the host reboot.');
      await refreshPreflight();
    } finally {
      setBusy(false);
    }
  };

  const cancel = async (): Promise<void> => {
    if (scheduled === null || !scheduled.cancellable || scheduled.state === 'executing') return;
    setBusy(true);
    setError(null);
    try {
      await cancelHostReboot(scheduled.operationId, csrfToken);
      setScheduled(null);
      setConfirmation('');
      setAcknowledged(false);
      await Promise.all([refreshPreflight(), onChanged()]);
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : 'Helix could not cancel the scheduled reboot.');
    } finally {
      setBusy(false);
    }
  };

  if (scheduled !== null) {
    const remaining = Math.max(0, Math.ceil((scheduled.executeAtUnixMs - now) / 1_000));
    return (
      <Dialog title="Host reboot scheduled" onClose={onClose}>
        <div class="reboot-countdown" aria-live="polite"><span>Rebooting in</span><strong>{remaining}s</strong><small>{formatTimestamp(scheduled.executeAtUnixMs)}</small></div>
        <div class="reboot-impact-note"><Icon name="warning" size={17} /><span><strong>Every service on {integration.hostname} will stop.</strong><small>Players, media streams, Helix, and other workloads will disconnect until Linux and their start-on-boot policies bring them back.</small></span></div>
        <InlineError message={error} />
        <div class="dialog-actions"><button class="button button--quiet" type="button" onClick={onClose}>Close</button><button class="button button--danger" type="button" disabled={busy || !scheduled.cancellable || scheduled.state === 'executing'} onClick={() => void cancel()}>{busy ? 'Cancelling…' : scheduled.state === 'executing' ? 'Reboot is executing' : 'Cancel reboot'}</button></div>
      </Dialog>
    );
  }

  const validation = validateHostReboot(integration.hostname, confirmation, acknowledged, delaySeconds, preflight);
  return (
    <Dialog title="Restart the whole host?" onClose={onClose} wide>
      <div class="reboot-dialog-copy"><p>This restarts Linux itself—not just Helix. The browser will disconnect and all running workloads will be interrupted.</p></div>
      <section class={`reboot-preflight ${preflight?.canSchedule === true ? 'is-clear' : ''}`}>
        <div><span><Icon name={preflight?.canSchedule === true ? 'check' : 'warning'} size={16} /><strong>{loading ? 'Checking the host…' : preflight?.canSchedule === true ? 'Preflight is clear' : 'Reboot is blocked'}</strong></span><button type="button" disabled={loading || busy} onClick={() => void refreshPreflight()}><Icon name="refresh" size={14} />Check again</button></div>
        {preflight !== null && <p>{preflight.activePlayers} active players · {preflight.activeServerCount} running servers · {preflight.activeJobsTotal} active jobs</p>}
        {preflight !== null && preflight.blockers.length > 0 && <ul>{preflight.blockers.map((blocker) => <li key={blocker.code}><strong>{blocker.code.replaceAll('_', ' ')}</strong><span>{blocker.message}</span></li>)}</ul>}
      </section>
      <div class="reboot-confirmation-grid">
        <label><span>Delay before reboot</span><div class="delay-input"><input type="number" min={10} max={300} step={1} value={delaySeconds} onInput={(event) => setDelaySeconds(event.currentTarget.valueAsNumber)} /><small>seconds</small></div><small>10–300 seconds gives you time to cancel.</small></label>
        <label><span>Type the hostname to confirm</span><input value={confirmation} autocomplete="off" autocapitalize="none" spellcheck={false} placeholder={integration.hostname} onInput={(event) => setConfirmation(event.currentTarget.value)} /><small>Exact value: <code>{integration.hostname}</code></small></label>
      </div>
      <label class="reboot-acknowledgement"><input type="checkbox" checked={acknowledged} onChange={(event) => setAcknowledged(event.currentTarget.checked)} /><span><strong>I understand this disrupts the entire host.</strong><small>Connected players and every other active service may lose unsaved work.</small></span></label>
      <InlineError message={error} />
      <div class="dialog-actions"><button class="button button--quiet" type="button" onClick={onClose}>Cancel</button><button class="button button--danger" type="button" disabled={busy || validation !== null} onClick={() => void submit()}>{busy ? 'Scheduling…' : `Reboot in ${Number.isFinite(delaySeconds) ? delaySeconds : '—'} seconds`}</button></div>
    </Dialog>
  );
}

const rebootWeekdays: readonly { id: RebootWeekday; label: string; short: string }[] = [
  { id: 'monday', label: 'Monday', short: 'Mon' },
  { id: 'tuesday', label: 'Tuesday', short: 'Tue' },
  { id: 'wednesday', label: 'Wednesday', short: 'Wed' },
  { id: 'thursday', label: 'Thursday', short: 'Thu' },
  { id: 'friday', label: 'Friday', short: 'Fri' },
  { id: 'saturday', label: 'Saturday', short: 'Sat' },
  { id: 'sunday', label: 'Sunday', short: 'Sun' },
];

export function validateRecurringHostReboot(
  integration: HostIntegration,
  weekdays: readonly RebootWeekday[],
  time: string,
  confirmation: string,
  acknowledged: boolean,
): string | null {
  if (integration.timezone === null) return 'Helix could not verify the Linux host timezone.';
  if (weekdays.length === 0) return 'Choose at least one weekday.';
  if (!/^(?:[01]\d|2[0-3]):[0-5]\d$/u.test(time)) return 'Choose a valid host time.';
  if (confirmation !== integration.hostname) return `Type ${integration.hostname} exactly to confirm this host.`;
  if (!acknowledged) return 'Acknowledge that each successful schedule will interrupt the whole host.';
  return null;
}

function RecurringHostRebootDialog({
  integration,
  csrfToken,
  onClose,
  onChanged,
}: {
  integration: HostIntegration;
  csrfToken: string;
  onClose: () => void;
  onChanged: () => Promise<void>;
}) {
  const existing = integration.recurringReboot.state === 'scheduled' || integration.recurringReboot.state === 'degraded'
    ? integration.recurringReboot
    : null;
  const [weekdays, setWeekdays] = useState<RebootWeekday[]>(existing?.weekdays ?? rebootWeekdays.map((day) => day.id));
  const [time, setTime] = useState(existing === null ? '05:00' : `${String(existing.hour).padStart(2, '0')}:${String(existing.minute).padStart(2, '0')}`);
  const [confirmation, setConfirmation] = useState('');
  const [acknowledged, setAcknowledged] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const validation = validateRecurringHostReboot(integration, weekdays, time, confirmation, acknowledged);

  const toggleDay = (day: RebootWeekday): void => {
    setWeekdays((current) => current.includes(day) ? current.filter((entry) => entry !== day) : rebootWeekdays.map((entry) => entry.id).filter((entry) => entry === day || current.includes(entry)));
  };

  const save = async (): Promise<void> => {
    if (validation !== null || integration.timezone === null) {
      setError(validation ?? 'Helix could not verify the Linux host timezone.');
      return;
    }
    const [hourText, minuteText] = time.split(':');
    setBusy(true);
    setError(null);
    try {
      await setRecurringHostReboot({
        weekdays,
        hour: Number(hourText),
        minute: Number(minuteText),
        timezone: integration.timezone,
        confirmationHostname: confirmation,
      }, csrfToken);
      await onChanged();
      onClose();
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : 'Helix could not save the recurring reboot schedule.');
    } finally {
      setBusy(false);
    }
  };

  const remove = async (): Promise<void> => {
    if (existing === null || confirmation !== integration.hostname) {
      setError(`Type ${integration.hostname} exactly before removing this schedule.`);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await deleteRecurringHostReboot(integration.hostname, csrfToken);
      await onChanged();
      onClose();
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : 'Helix could not remove the recurring reboot schedule.');
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog title={existing === null ? 'Set a recurring host reboot' : 'Recurring host reboot'} onClose={onClose} wide>
      <div class="recurring-reboot-summary">
        <Icon name="clock" size={18} />
        <span><strong>Runs in {integration.timezone ?? 'an unverified host timezone'}</strong><small>Every run checks connected players, active Helix jobs, and manager health again. A blocked run is safely skipped, and missed runs never catch up later.</small></span>
      </div>
      {existing !== null && <div class={`recurring-reboot-current recurring-reboot-current--${existing.state}`}><span><strong>{existing.state === 'scheduled' ? 'Schedule active' : 'Schedule needs attention'}</strong><small>{existing.nextAtUnixMs === null ? 'No next run was verified.' : `Next safety check: ${formatTimestamp(existing.nextAtUnixMs)}`}</small></span><small>{existing.calendarExpression}</small></div>}
      {integration.recurringReboot.state === 'unavailable' && <InlineError message={`The saved schedule cannot be managed safely: ${integration.recurringReboot.reason.replaceAll('_', ' ')}.`} />}
      <fieldset class="recurring-reboot-days" disabled={busy || integration.recurringReboot.state === 'unavailable'}><legend>Days</legend><div>{rebootWeekdays.map((day) => <button key={day.id} type="button" class={weekdays.includes(day.id) ? 'is-active' : ''} aria-pressed={weekdays.includes(day.id)} title={day.label} onClick={() => toggleDay(day.id)}>{day.short}</button>)}</div><button class="recurring-days-toggle" type="button" onClick={() => setWeekdays(weekdays.length === 7 ? [] : rebootWeekdays.map((day) => day.id))}>{weekdays.length === 7 ? 'Clear days' : 'Every day'}</button></fieldset>
      <div class="reboot-confirmation-grid">
        <label><span>Host time</span><input type="time" required value={time} disabled={busy || integration.timezone === null} onInput={(event) => setTime(event.currentTarget.value)} /><small>{integration.timezone === null ? 'Linux timezone is unavailable.' : `Linux timezone: ${integration.timezone}`}</small></label>
        <label><span>Type the hostname to confirm</span><input value={confirmation} autocomplete="off" autocapitalize="none" spellcheck={false} placeholder={integration.hostname} onInput={(event) => setConfirmation(event.currentTarget.value)} /><small>Exact value: <code>{integration.hostname}</code></small></label>
      </div>
      <label class="reboot-acknowledgement"><input type="checkbox" checked={acknowledged} onChange={(event) => setAcknowledged(event.currentTarget.checked)} /><span><strong>I understand successful runs restart the entire host.</strong><small>Linux startup and each workload’s own start-on-boot policy determine what returns afterward.</small></span></label>
      <InlineError message={error} />
      <div class="dialog-actions dialog-actions--split">{existing !== null ? <button class="button button--danger-quiet" type="button" disabled={busy || confirmation !== integration.hostname} onClick={() => void remove()}>{busy ? 'Working…' : 'Remove schedule'}</button> : <span />}<span><button class="button button--quiet" type="button" disabled={busy} onClick={onClose}>Cancel</button><button class="button button--danger" type="button" disabled={busy || validation !== null || integration.recurringReboot.state === 'unavailable'} onClick={() => void save()}>{busy ? 'Saving…' : existing === null ? 'Create schedule' : 'Save schedule'}</button></span></div>
    </Dialog>
  );
}

function HostIntegrationSettings({
  resource,
  user,
  csrfToken,
  onRefresh,
}: {
  resource: DashboardResource<HostIntegration>;
  user: AuthenticatedUser;
  csrfToken: string;
  onRefresh: () => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [rebootOpen, setRebootOpen] = useState(false);
  const [recurringRebootOpen, setRecurringRebootOpen] = useState(false);
  const [optimisticStartOnBoot, setOptimisticStartOnBoot] = useState<boolean | null>(null);
  const [confirmedStartOnBoot, setConfirmedStartOnBoot] = useState<boolean | null>(null);
  const integration = resource.data;
  const canWrite = user.capabilities.includes('system.settings.write');
  const canPower = user.capabilities.includes('system.power');
  const displayedStartOnBoot = optimisticStartOnBoot ?? confirmedStartOnBoot ?? integration?.startOnBoot.enabled ?? null;

  useEffect(() => {
    if (confirmedStartOnBoot !== null && integration?.startOnBoot.enabled === confirmedStartOnBoot) {
      setConfirmedStartOnBoot(null);
    }
  }, [confirmedStartOnBoot, integration?.startOnBoot.enabled]);

  const toggle = async (): Promise<void> => {
    if (integration === null || integration.startOnBoot.state === 'unavailable' || busy || !canWrite) return;
    const target = displayedStartOnBoot !== true;
    setBusy(true);
    setOptimisticStartOnBoot(target);
    setError(null);
    setNotice(null);
    try {
      const result = await setHelixStartOnBoot(target, csrfToken);
      setConfirmedStartOnBoot(result.enabled);
      setOptimisticStartOnBoot(null);
      setBusy(false);
      setNotice(`${result.enabled ? 'Enabled' : 'Disabled'} for ${result.containers.length} Helix container${result.containers.length === 1 ? '' : 's'}. Running containers were not changed.`);
      void onRefresh().catch((refreshError: unknown) => {
        setError(refreshError instanceof Error ? refreshError.message : 'Helix updated the setting, but could not refresh its host state.');
      });
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : 'Helix could not update start on boot.');
    } finally {
      setBusy(false);
      setOptimisticStartOnBoot(null);
    }
  };

  const serviceRow = (name: string, service: HostIntegration['services']['docker']) => <div class="host-integration-row"><span><i class={`status-dot status-dot--${service.active ? 'good' : 'idle'}`} /><strong>{name}</strong></span><span>{service.activeState}</span><small>{service.enabled ? 'Starts with Linux' : `Boot state: ${service.enabledState}`}</small></div>;

  return (
    <>
      <section class="settings-card settings-card--host">
        <div class="settings-card__head"><div><Icon name="host" /><span><h2>Host integration</h2><p>Real Linux service and Docker restart state.</p></span></div><InfoTip text="Helix uses a privileged broker for bounded host actions and Docker restart policies for its two dashboard containers. This does not control Minecraft server start-on-boot settings." /></div>
        <InlineError message={resource.error ?? error} />
        {integration === null ? <div class="host-integration-empty"><Icon name="refresh" class={resource.phase === 'loading' ? 'is-spinning' : undefined} /><span>{resource.phase === 'loading' ? 'Reading Linux and Docker state…' : 'Host integration data is unavailable.'}</span></div> : <>
          <div class="host-integration-list">{serviceRow('Docker', integration.services.docker)}{serviceRow('Helix broker', integration.services.helixPrivd)}</div>
          <div class="host-container-policies">{([['Dashboard', integration.containers.dashboard], ['Gateway', integration.containers.gateway]] as const).map(([label, container]) => <div key={label}><span><i class={`status-dot status-dot--${container?.running === true ? 'good' : 'idle'}`} /><strong>{label}</strong></span><span>{container === null ? 'Not found' : container.running ? container.health ?? 'Running' : 'Stopped'}</span><small>Restart policy: <code>{container?.restartPolicy ?? 'unavailable'}</code></small></div>)}</div>
          <div class="host-boot-control"><div><span>Helix dashboard after boot</span><strong>{busy && optimisticStartOnBoot !== null ? optimisticStartOnBoot ? 'Enabling…' : 'Disabling…' : integration.startOnBoot.state === 'unavailable' ? 'Unavailable' : displayedStartOnBoot === true ? 'Enabled' : displayedStartOnBoot === false ? 'Disabled' : 'Mixed policies'}</strong><small>This is the Helix dashboard and gateway only, not Minecraft or other game servers. It only changes what Docker does after Linux restarts. It does not start or stop anything right now.</small></div><button class="switch-button" role="switch" aria-checked={displayedStartOnBoot ?? 'mixed'} aria-busy={busy} type="button" disabled={busy || !canWrite || integration.startOnBoot.state === 'unavailable'} onClick={() => void toggle()}><i /><span>{busy ? optimisticStartOnBoot ? 'Enabling…' : 'Disabling…' : displayedStartOnBoot === true ? 'On' : 'Off'}</span></button></div>
          <div class="host-policy-caveat"><Icon name="info" size={15} /><span>{integration.startOnBoot.note ?? 'Docker restart policy is the source of truth.'} Container recreation or a future Compose change may reapply a different policy.</span></div>
          {notice !== null && <div class="host-integration-notice" role="status"><Icon name="check" size={14} />{notice}</div>}
          {!canWrite && <div class="host-integration-notice"><Icon name="info" size={14} />This account can view integration state but cannot change it.</div>}
        </>}
      </section>
      <section class="settings-card settings-card--danger">
        <div class="settings-card__head"><div><Icon name="warning" /><span><h2>Whole-host reboot</h2><p>Restart Linux and every workload on this machine.</p></span></div><InfoTip text="Helix runs a fresh safety preflight before scheduling. Active players, jobs, or unavailable player checks block the reboot." /></div>
        <div class="host-danger-body"><div><strong>{integration?.scheduledReboot.state === 'none' || integration === null ? 'No immediate reboot scheduled' : integration.scheduledReboot.state === 'scheduled' ? `Reboot scheduled for ${formatTimestamp(integration.scheduledReboot.executeAtUnixMs)}` : 'Reboot is executing'}</strong><span>This disconnects Helix, all players, media streams, and every other active host service.</span></div><div class="host-danger-actions"><button class="button button--danger" type="button" disabled={integration === null || !canPower} onClick={() => setRebootOpen(true)}>{integration?.scheduledReboot.state !== 'none' && integration !== null ? 'View reboot' : 'Reboot now'}</button><button class="button button--quiet" type="button" disabled={integration === null || !canPower} onClick={() => setRecurringRebootOpen(true)}>{integration?.recurringReboot.state === 'scheduled' || integration?.recurringReboot.state === 'degraded' ? 'Edit recurring schedule' : 'Recurring schedule'}</button></div></div>
        {integration !== null && <div class="host-recurring-state"><Icon name="clock" size={15} /><span>{integration.recurringReboot.state === 'none' ? 'No recurring reboot schedule.' : integration.recurringReboot.state === 'unavailable' ? 'A recurring schedule exists but cannot be verified safely.' : integration.recurringReboot.nextAtUnixMs === null ? 'Recurring schedule exists; next run could not be verified.' : `Next recurring safety check: ${formatTimestamp(integration.recurringReboot.nextAtUnixMs)} (${integration.recurringReboot.timezone}).`}</span></div>}
        {!canPower && <div class="host-integration-notice"><Icon name="info" size={14} />This account does not have whole-host power permission.</div>}
      </section>
      {rebootOpen && integration !== null && <HostRebootDialog integration={integration} csrfToken={csrfToken} onClose={() => setRebootOpen(false)} onChanged={onRefresh} />}
      {recurringRebootOpen && integration !== null && <RecurringHostRebootDialog integration={integration} csrfToken={csrfToken} onClose={() => setRecurringRebootOpen(false)} onChanged={onRefresh} />}
    </>
  );
}

function dismissedNoticeLabel(id: string): string {
  if (id === 'storage-space-intro') return 'Storage space-analyzer intro';
  if (id.startsWith('capacity:')) return `Full-disk warning for ${id.slice('capacity:'.length)}`;
  return 'A dashboard notice';
}

function HelixDataSettings({
  servers,
  csrfToken,
  canManage,
  onRefresh,
}: {
  servers: ManagedServer[];
  csrfToken: string;
  canManage: boolean;
  onRefresh: () => Promise<void>;
}) {
  const helixServers = servers.filter((server) => server.manager === 'helix');
  const imported = servers.filter((server) => server.manager !== 'helix');
  const [removed, setRemoved] = useState<TrashedNativeServerCatalog | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [pendingTrash, setPendingTrash] = useState<ManagedServer | null>(null);
  const [pendingPurge, setPendingPurge] = useState<TrashedNativeServer | null>(null);
  const [confirmName, setConfirmName] = useState('');
  const [dismissedIds, setDismissedIds] = useState(listDismissedIds);
  const [forgottenImported, setForgottenImported] = useState(readForgottenImportedServers);
  const [removedEpoch, setRemovedEpoch] = useState(0);
  const forgottenServers = imported.filter((server) => forgottenImported.includes(server.id));

  useEffect(() => {
    const refreshNotices = (): void => setDismissedIds(listDismissedIds());
    window.addEventListener(DISMISSALS_CHANGED_EVENT, refreshNotices);
    return () => window.removeEventListener(DISMISSALS_CHANGED_EVENT, refreshNotices);
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    void getTrashedNativeServers(csrfToken, controller.signal)
      .then((catalog) => {
        setRemoved(catalog);
        setError(null);
      })
      .catch((reason: unknown) => {
        if (!controller.signal.aborted) setError(reason instanceof Error ? reason.message : 'Helix could not list recoverable servers.');
      });
    return () => controller.abort();
  }, [csrfToken, servers, removedEpoch]);

  const toggleBoot = async (server: ManagedServer): Promise<void> => {
    if (!canManage || busyId !== null) return;
    setBusyId(server.id);
    setError(null);
    try {
      await setNativeStartOnBoot(server.id, !server.startOnBoot, csrfToken);
      await onRefresh();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Helix could not change start-on-boot.');
    } finally {
      setBusyId(null);
    }
  };

  const trash = async (): Promise<void> => {
    if (pendingTrash === null || confirmName !== pendingTrash.name || !canManage) return;
    setBusyId(pendingTrash.id);
    setError(null);
    try {
      await trashNativeServer(pendingTrash.id, confirmName, csrfToken);
      setPendingTrash(null);
      setConfirmName('');
      await onRefresh();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Helix could not move that server to recoverable trash.');
    } finally {
      setBusyId(null);
    }
  };

  const restore = async (trashId: string): Promise<void> => {
    if (!canManage || busyId !== null) return;
    setBusyId(trashId);
    setError(null);
    try {
      await restoreTrashedNativeServer(trashId, csrfToken);
      await onRefresh();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Helix could not restore that server.');
    } finally {
      setBusyId(null);
    }
  };

  const purge = async (): Promise<void> => {
    if (pendingPurge === null || confirmName !== pendingPurge.name || !canManage) return;
    setBusyId(pendingPurge.trashId);
    setError(null);
    try {
      await purgeTrashedNativeServer(pendingPurge.trashId, confirmName, csrfToken);
      setPendingPurge(null);
      setConfirmName('');
      setRemovedEpoch((epoch) => epoch + 1);
      await onRefresh();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Helix could not permanently delete that server.');
    } finally {
      setBusyId(null);
    }
  };

  const showForgottenImported = (id: string): void => {
    const next = rememberImportedServer(id, readHiddenImportedServers(), forgottenImported);
    setForgottenImported(next.forgotten);
  };

  const showNoticesAgain = (): void => {
    clearDismissals();
    setDismissedIds([]);
  };

  return (
    <section class="settings-card settings-card--helix-data">
      <div class="settings-card__head">
        <div>
          <Icon name="folder" />
          <span>
            <h2>Helix data</h2>
            <p>Native servers Helix owns, recoverable trash, forgotten AMP connections in this browser, and notices this browser has hidden.</p>
          </span>
        </div>
        <InfoTip text="Imported AMP or other connections stay owned by those managers. Removing a native server moves its files into recoverable trash; Delete forever from that list erases them. Start after the host boots is also on the Servers page when you create a server." />
      </div>
      <div class="helix-data-body">
        <div class="helix-data-summary">
          <div>
            <strong>{helixServers.length}</strong>
            <span>Native servers</span>
            <small>Created and owned by Helix</small>
          </div>
          <div>
            <strong>{imported.length}</strong>
            <span>Imported connections</span>
            <small>AMP or other managers Helix can see. Helix does not own those files.</small>
          </div>
          <div>
            <strong>{removed?.servers.length ?? '—'}</strong>
            <span>Recoverable trash</span>
            <small>Removed native servers you can restore or delete forever</small>
          </div>
        </div>

        <div class="helix-data-section">
          <h3>Native servers</h3>
          <p>
            Each native server can come back by itself after Linux or Docker restarts. That choice is on when you create a server, and you can change it here or on the Servers page. It does not start or stop the server right now.
          </p>
          {helixServers.length === 0 ? (
            <p class="helix-data-empty">No native Helix servers yet. Create one from Servers → New server. Start after the host boots is on by default there.</p>
          ) : (
            <ul class="helix-data-list">
              {helixServers.map((server) => {
                const bootCopyId = `helix-data-boot-${server.id.replace(/[^a-zA-Z0-9_-]+/g, '-')}`;
                const saving = busyId === server.id;
                return (
                  <li key={server.id} class="helix-data-server">
                    <header>
                      <GameMark game={gameMarkForSoftware(server.software, server.kind) ?? 'minecraft'} size={28} />
                      <div>
                        <strong>{server.name}</strong>
                        <small>
                          {server.software} · {serverStatusLabel(server.status)} · port {server.gamePort || '—'}
                        </small>
                      </div>
                    </header>
                    <div class="helix-data-boot">
                      <div>
                        <span>{START_WITH_HOST_TITLE}</span>
                        <strong>{server.startOnBoot ? 'On' : 'Off'}</strong>
                        <small id={bootCopyId}>{START_WITH_HOST_DETAIL}</small>
                      </div>
                      <button
                        class="switch-button"
                        role="switch"
                        type="button"
                        disabled={!canManage || busyId !== null}
                        aria-checked={server.startOnBoot}
                        aria-describedby={bootCopyId}
                        aria-label={`${START_WITH_HOST_TITLE} for ${server.name}`}
                        onClick={() => void toggleBoot(server)}
                      >
                        <i />
                        <span>{saving ? 'Saving…' : server.startOnBoot ? 'On' : 'Off'}</span>
                      </button>
                    </div>
                    <div class="helix-data-server-actions">
                      <a class="button button--quiet" href={serverDetailHash(server.id)}>Open in Servers</a>
                      <button
                        class="button button--quiet"
                        type="button"
                        disabled={!canManage || busyId !== null}
                        onClick={() => {
                          setPendingTrash(server);
                          setConfirmName('');
                        }}
                      >
                        Remove…
                      </button>
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        {(removed?.servers.length ?? 0) > 0 && (
          <div class="helix-data-section helix-data-trash">
            <h3>Recoverable trash</h3>
            <p>These native servers were removed from Helix. Restore brings the files back onto the Servers page. Delete forever erases the world files, Helix backups, and console history. This is not an off-host backup.</p>
            <ul>
              {removed?.servers.map((item) => (
                <li key={item.trashId}>
                  <span>
                    {item.name}
                    <small>{item.software} · {formatTimestamp(item.trashedAtUnixMs)}</small>
                  </span>
                  <div class="helix-data-trash-actions">
                    <button class="button button--quiet" type="button" disabled={!canManage || busyId !== null} onClick={() => void restore(item.trashId)}>
                      Restore
                    </button>
                    <button
                      class="button button--danger"
                      type="button"
                      disabled={!canManage || busyId !== null}
                      onClick={() => {
                        setPendingPurge(item);
                        setConfirmName('');
                      }}
                    >
                      Delete forever
                    </button>
                  </div>
                </li>
              ))}
            </ul>
            {removed?.policy.note !== undefined && removed.policy.note.length > 0 && <p>{removed.policy.note}</p>}
          </div>
        )}

        {forgottenServers.length > 0 && (
          <div class="helix-data-section helix-data-trash">
            <h3>Forgotten AMP connections</h3>
            <p>Forgotten in this browser only. Helix did not stop or delete the AMP instance. Show on Servers puts it back on the Servers list here.</p>
            <ul>
              {forgottenServers.map((server) => (
                <li key={server.id}>
                  <span>
                    {server.name}
                    <small>{server.software} · AMP connection</small>
                  </span>
                  <button class="button button--quiet" type="button" onClick={() => showForgottenImported(server.id)}>
                    Show on Servers
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}

        <div class="helix-data-section">
          <h3>Dismissed notices</h3>
          <p>
            If you closed a full-disk warning on Overview, a notice in the bell menu, or the Storage space-analyzer intro, Helix remembers that in this browser only. Other browsers and other people on this dashboard still see those banners. Show them again brings them back.
          </p>
          <div class="helix-data-notices">
            <div>
              <strong>{dismissedIds.length === 0 ? 'Nothing hidden' : `${dismissedIds.length} hidden in this browser`}</strong>
              {dismissedIds.length === 0 ? (
                <small>No Overview, storage, or bell notices are hidden here.</small>
              ) : (
                <ul>
                  {dismissedIds.map((id) => (
                    <li key={id}>{dismissedNoticeLabel(id)}</li>
                  ))}
                </ul>
              )}
            </div>
            <button class="button button--quiet" type="button" disabled={dismissedIds.length === 0} onClick={showNoticesAgain}>
              Show them again
            </button>
          </div>
        </div>

        <InlineError message={error} />
        {!canManage && (
          <div class="host-integration-notice">
            <Icon name="info" size={14} />
            This account can view Helix data but cannot remove, restore, or permanently delete servers.
          </div>
        )}
      </div>
      {pendingTrash !== null && (
        <Dialog title={`Remove ${pendingTrash.name}?`} onClose={() => setPendingTrash(null)}>
          <p class="dialog-intro">This moves the native server into recoverable trash. Type the exact server name to confirm.</p>
          <label class="field field--wide"><span>Server name</span><input value={confirmName} onInput={(event) => setConfirmName(event.currentTarget.value)} autocomplete="off" /></label>
          <div class="dialog-actions">
            <button class="button button--quiet" type="button" onClick={() => setPendingTrash(null)}>Cancel</button>
            <button class="button button--danger" type="button" disabled={confirmName !== pendingTrash.name || busyId !== null} onClick={() => void trash()}>{busyId !== null ? 'Removing…' : 'Move to trash'}</button>
          </div>
        </Dialog>
      )}
      {pendingPurge !== null && (
        <Dialog title={`Delete ${pendingPurge.name} forever?`} onClose={() => busyId === null && setPendingPurge(null)}>
          <p class="dialog-intro">This permanently erases the recovered world files, Helix backups, and console history. Type the exact server name to confirm.</p>
          <label class="field field--wide"><span>Server name</span><input value={confirmName} onInput={(event) => setConfirmName(event.currentTarget.value)} autocomplete="off" disabled={busyId !== null} /></label>
          <div class="dialog-actions">
            <button class="button button--quiet" type="button" disabled={busyId !== null} onClick={() => setPendingPurge(null)}>Cancel</button>
            <button class="button button--danger" type="button" disabled={confirmName !== pendingPurge.name || busyId !== null} onClick={() => void purge()}>{busyId !== null ? 'Deleting…' : 'Delete forever'}</button>
          </div>
        </Dialog>
      )}
    </section>
  );
}

export function DashboardSettingsPage({
  user,
  csrfToken,
  theme,
  refreshIntervalMs,
  navigationOrder,
  hiddenPages,
  colors,
  serversEnabled,
  preferenceSyncStatus,
  hostIntegration,
  servers,
  onThemeChange,
  onRefreshIntervalChange,
  onNavigationOrderChange,
  onHiddenPagesChange,
  onColorsChange,
  onServersEnabledChange,
  onAccountUpdated,
  onHostIntegrationRefresh,
}: {
  user: AuthenticatedUser;
  csrfToken: string;
  theme: ThemePreference;
  refreshIntervalMs: RefreshIntervalMs;
  navigationOrder: readonly PrimaryDashboardSectionId[];
  hiddenPages: readonly PrimaryDashboardSectionId[];
  colors: DashboardColors;
  serversEnabled: boolean;
  preferenceSyncStatus: 'loading' | 'synced' | 'saving' | 'local';
  hostIntegration: DashboardResource<HostIntegration>;
  servers: ManagedServer[];
  onThemeChange: (theme: ThemePreference) => void;
  onRefreshIntervalChange: (value: RefreshIntervalMs) => void;
  onNavigationOrderChange: (value: PrimaryDashboardSectionId[]) => void;
  onHiddenPagesChange: (value: PrimaryDashboardSectionId[]) => void;
  onColorsChange: (value: DashboardColors) => void;
  onServersEnabledChange: (value: boolean) => void;
  onAccountUpdated: () => void;
  onHostIntegrationRefresh: () => Promise<void>;
}) {
  return (
    <div class="page page--dashboard-settings">
      <div class="page-head"><div><h1>Settings</h1><p>Dashboard behavior, layout, appearance, and owner security.</p></div></div>
      <div class={`settings-sync-state settings-sync-state--${preferenceSyncStatus}`}><Icon name={preferenceSyncStatus === 'synced' ? 'check' : preferenceSyncStatus === 'local' ? 'warning' : 'refresh'} size={15} /><span>{preferenceSyncStatus === 'synced' ? 'Dashboard preferences are synced' : preferenceSyncStatus === 'saving' ? 'Saving dashboard preferences…' : preferenceSyncStatus === 'loading' ? 'Loading dashboard preferences…' : 'Preference service unavailable · using the browser copy'}</span></div>
      <div class="settings-page-grid">
        <section class="settings-card">
          <div class="settings-card__head"><div><Icon name="performance" /><span><h2>Live data</h2><p>Control how often visible host and server statistics update.</p></span></div><InfoTip text="Faster refreshes query the host more often. Polling pauses while this tab is hidden." /></div>
          <label class="settings-select-row"><span><strong>Refresh interval</strong><small>Applies throughout the dashboard</small></span><select value={refreshIntervalMs} onChange={(event) => onRefreshIntervalChange(Number(event.currentTarget.value) as RefreshIntervalMs)}>{refreshIntervalOptions.map((option) => <option key={option} value={option}>{intervalLabel(option)}</option>)}</select></label>
        </section>
        <section class="settings-card">
          <div class="settings-card__head"><div><Icon name="servers" /><span><h2>Game servers</h2><p>Show or hide the Servers page without touching running game containers.</p></span></div><InfoTip text="Owner setup can skip Servers. Turning this on later is the same as choosing it during setup. Hiding the page does not stop Minecraft, V Rising, or imported servers." /></div>
          <div class="host-boot-control">
            <div>
              <span>Servers dashboard</span>
              <strong>{serversEnabled ? 'Enabled' : 'Hidden'}</strong>
              <small>Native Minecraft, V Rising, Valheim, Terraria, AMP imports, and port pools live here. Existing servers keep running if you hide the page.</small>
            </div>
            <button
              class="switch-button"
              role="switch"
              aria-checked={serversEnabled}
              type="button"
              onClick={() => onServersEnabledChange(!serversEnabled)}
            >
              <i />
              <span>{serversEnabled ? 'On' : 'Off'}</span>
            </button>
          </div>
        </section>
        <CatalogsSettings user={user} csrfToken={csrfToken} />
        <section class="settings-card">
          <div class="settings-card__head"><div><Icon name="moon" /><span><h2>Appearance</h2><p>Choose the contrast that works best on this screen.</p></span></div></div>
          <div class="theme-choice-grid">{(['system', 'midnight', 'oled', 'light'] as const).map((option) => <button key={option} class={theme === option ? 'is-active' : ''} type="button" aria-pressed={theme === option} onClick={() => onThemeChange(option)}><span class={`theme-preview theme-preview--${option}`}><i /><i /><i /></span><strong>{themeLabels[option]}</strong><small>{option === 'system' ? 'Follow this device' : option === 'oled' ? 'True black surfaces' : `${themeLabels[option]} palette`}</small>{theme === option && <Icon name="check" size={14} />}</button>)}</div>
          <div class="appearance-color-center"><div><strong>Color control center</strong><span>Override individual theme colors, or leave them on Theme.</span></div><div class="appearance-color-grid">{([['accent', 'Accent', '#d7f64d'], ['text', 'Text', '#f1f0eb'], ['surface', 'Panels', '#15181d']] as const).map(([key, label, fallback]) => <label key={key}><span>{label}</span><span><input type="color" value={colors[key] || fallback} onInput={(event) => onColorsChange({ ...colors, [key]: event.currentTarget.value.toLowerCase() })} /><button type="button" disabled={colors[key].length === 0} onClick={() => onColorsChange({ ...colors, [key]: '' })}>{colors[key].length === 0 ? 'Theme' : 'Reset'}</button></span></label>)}</div><div class="appearance-accent-presets"><span>Accent presets</span>{([['#d7f64d', 'Lime'], ['#5fd7ff', 'Sky'], ['#a98bff', 'Violet'], ['#ffb454', 'Amber'], ['#ff6f91', 'Rose']] as const).map(([color, label]) => <button key={color} type="button" title={label} aria-label={`${label} accent`} style={{ background: color }} onClick={() => onColorsChange({ ...colors, accent: color })} />)}<button class="button button--quiet" type="button" disabled={colors.accent.length === 0 && colors.text.length === 0 && colors.surface.length === 0} onClick={() => onColorsChange({ accent: '', text: '', surface: '' })}>Reset all colors</button></div></div>
        </section>
        <section class="settings-card settings-card--navigation">
          <div class="settings-card__head"><div><Icon name="menu" /><span><h2>Navigation</h2><p>Arrange, hide, or add the main pages in the sidebar and mobile bar.</p></span></div><InfoTip text="Settings stays pinned at the bottom. Globe is off the sidebar until you add it. Hidden pages keep their place in the order so Add puts them back where they were. This follows the owner account across browsers." /></div>
          <ol class="navigation-order-list">
            {visibleDashboardSections(navigationOrder, hiddenPages, serversEnabled).map((section, index, visible) => {
              const item = navigationLabels[section];
              return (
                <li key={section}>
                  <span><Icon name={item.icon} size={16} /><strong>{item.label}</strong></span>
                  <div>
                    <button type="button" disabled={index === 0} onClick={() => onNavigationOrderChange(moveVisibleNavigationItem(navigationOrder, hiddenPages, serversEnabled, section, -1))} aria-label={`Move ${item.label} up`}><Icon name="chevron" size={14} class="icon--up" /></button>
                    <button type="button" disabled={index === visible.length - 1} onClick={() => onNavigationOrderChange(moveVisibleNavigationItem(navigationOrder, hiddenPages, serversEnabled, section, 1))} aria-label={`Move ${item.label} down`}><Icon name="chevron" size={14} class="icon--down" /></button>
                    <button type="button" onClick={() => (section === 'servers' ? onServersEnabledChange(false) : onHiddenPagesChange(hideDashboardPage(hiddenPages, section)))} aria-label={`Hide ${item.label}`}><Icon name="trash" size={14} /></button>
                  </div>
                </li>
              );
            })}
          </ol>
          {addableDashboardSections(navigationOrder, hiddenPages, serversEnabled).length > 0 && (
            <div class="navigation-add-catalog">
              <span>Add a page</span>
              {addableDashboardSections(navigationOrder, hiddenPages, serversEnabled).map((section) => {
                const item = navigationLabels[section];
                return (
                  <button type="button" key={section} onClick={() => (section === 'servers' ? onServersEnabledChange(true) : onHiddenPagesChange(showDashboardPage(hiddenPages, section)))}>
                    <Icon name="plus" size={14} />
                    <Icon name={item.icon} size={15} />
                    {item.label}
                  </button>
                );
              })}
            </div>
          )}
          <div class="settings-card__foot"><span>{preferenceSyncStatus === 'local' ? 'Browser fallback active' : 'Synced through Helix'}</span><button class="button button--quiet" type="button" onClick={() => { onNavigationOrderChange([...primaryDashboardSections]); onHiddenPagesChange([...defaultHiddenPages]); if (!serversEnabled) onServersEnabledChange(true); }}>Reset pages</button></div>
        </section>
        <HelixDataSettings servers={servers} csrfToken={csrfToken} canManage={user.capabilities.includes('games.manage')} onRefresh={onHostIntegrationRefresh} />
        <HostIntegrationSettings resource={hostIntegration} user={user} csrfToken={csrfToken} onRefresh={onHostIntegrationRefresh} />
        <AccountSettings user={user} csrfToken={csrfToken} onAccountUpdated={onAccountUpdated} />
      </div>
    </div>
  );
}
