import { useEffect, useState } from 'preact/hooks';
import { updateAccount } from './api';
import type { DashboardResource } from './dashboard-model';
import {
  moveNavigationItem,
  primaryDashboardSections,
  refreshIntervalOptions,
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

const navigationLabels: Record<PrimaryDashboardSectionId, { label: string; icon: IconName }> = {
  overview: { label: 'Overview', icon: 'overview' },
  home: { label: 'Home', icon: 'home' },
  storage: { label: 'Storage', icon: 'storage' },
  network: { label: 'Network', icon: 'network' },
  host: { label: 'Host', icon: 'host' },
  terminal: { label: 'Terminal', icon: 'terminal' },
  servers: { label: 'Servers', icon: 'servers' },
  hooks: { label: 'Hooks', icon: 'hooks' },
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
  const integration = resource.data;
  const canWrite = user.capabilities.includes('system.settings.write');
  const canPower = user.capabilities.includes('system.power');

  const toggle = async (): Promise<void> => {
    if (integration === null || integration.startOnBoot.state === 'unavailable' || busy || !canWrite) return;
    const target = integration.startOnBoot.enabled !== true;
    setBusy(true);
    setOptimisticStartOnBoot(target);
    setError(null);
    setNotice(null);
    try {
      const result = await setHelixStartOnBoot(target, csrfToken);
      setNotice(`${result.enabled ? 'Enabled' : 'Disabled'} for ${result.containers.length} Helix container${result.containers.length === 1 ? '' : 's'}. Running containers were not changed.`);
      await onRefresh();
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
          <div class="host-boot-control"><div><span>Helix starts after boot</span><strong>{busy && optimisticStartOnBoot !== null ? optimisticStartOnBoot ? 'Enabling…' : 'Disabling…' : integration.startOnBoot.state === 'mixed' ? 'Mixed policies' : integration.startOnBoot.state === 'unavailable' ? 'Unavailable' : integration.startOnBoot.enabled ? 'Enabled' : 'Disabled'}</strong><small>This changes restart policy only. It does not start or stop containers now.</small></div><button class="switch-button" role="switch" aria-checked={optimisticStartOnBoot ?? integration.startOnBoot.enabled ?? 'mixed'} aria-busy={busy} type="button" disabled={busy || !canWrite || integration.startOnBoot.state === 'unavailable'} onClick={() => void toggle()}><i /><span>{busy ? optimisticStartOnBoot ? 'Enabling…' : 'Disabling…' : integration.startOnBoot.enabled === true ? 'On' : 'Off'}</span></button></div>
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

export function DashboardSettingsPage({
  user,
  csrfToken,
  theme,
  refreshIntervalMs,
  navigationOrder,
  colors,
  preferenceSyncStatus,
  hostIntegration,
  onThemeChange,
  onRefreshIntervalChange,
  onNavigationOrderChange,
  onColorsChange,
  onAccountUpdated,
  onHostIntegrationRefresh,
}: {
  user: AuthenticatedUser;
  csrfToken: string;
  theme: ThemePreference;
  refreshIntervalMs: RefreshIntervalMs;
  navigationOrder: readonly PrimaryDashboardSectionId[];
  colors: DashboardColors;
  preferenceSyncStatus: 'loading' | 'synced' | 'saving' | 'local';
  hostIntegration: DashboardResource<HostIntegration>;
  onThemeChange: (theme: ThemePreference) => void;
  onRefreshIntervalChange: (value: RefreshIntervalMs) => void;
  onNavigationOrderChange: (value: PrimaryDashboardSectionId[]) => void;
  onColorsChange: (value: DashboardColors) => void;
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
          <div class="settings-card__head"><div><Icon name="moon" /><span><h2>Appearance</h2><p>Choose the contrast that works best on this screen.</p></span></div></div>
          <div class="theme-choice-grid">{(['system', 'midnight', 'oled', 'light'] as const).map((option) => <button key={option} class={theme === option ? 'is-active' : ''} type="button" aria-pressed={theme === option} onClick={() => onThemeChange(option)}><span class={`theme-preview theme-preview--${option}`}><i /><i /><i /></span><strong>{themeLabels[option]}</strong><small>{option === 'system' ? 'Follow this device' : option === 'oled' ? 'True black surfaces' : `${themeLabels[option]} palette`}</small>{theme === option && <Icon name="check" size={14} />}</button>)}</div>
          <div class="appearance-color-center"><div><strong>Color control center</strong><span>Override individual theme colors, or leave them on Theme.</span></div><div class="appearance-color-grid">{([['accent', 'Accent', '#d7f64d'], ['text', 'Text', '#f1f0eb'], ['surface', 'Panels', '#15181d']] as const).map(([key, label, fallback]) => <label key={key}><span>{label}</span><span><input type="color" value={colors[key] || fallback} onInput={(event) => onColorsChange({ ...colors, [key]: event.currentTarget.value.toLowerCase() })} /><button type="button" disabled={colors[key].length === 0} onClick={() => onColorsChange({ ...colors, [key]: '' })}>{colors[key].length === 0 ? 'Theme' : 'Reset'}</button></span></label>)}</div><div class="appearance-accent-presets"><span>Accent presets</span>{([['#d7f64d', 'Lime'], ['#5fd7ff', 'Sky'], ['#a98bff', 'Violet'], ['#ffb454', 'Amber'], ['#ff6f91', 'Rose']] as const).map(([color, label]) => <button key={color} type="button" title={label} aria-label={`${label} accent`} style={{ background: color }} onClick={() => onColorsChange({ ...colors, accent: color })} />)}<button class="button button--quiet" type="button" disabled={colors.accent.length === 0 && colors.text.length === 0 && colors.surface.length === 0} onClick={() => onColorsChange({ accent: '', text: '', surface: '' })}>Reset all colors</button></div></div>
        </section>
        <section class="settings-card settings-card--navigation">
          <div class="settings-card__head"><div><Icon name="menu" /><span><h2>Navigation order</h2><p>Arrange the main pages in the sidebar and mobile bar.</p></span></div><InfoTip text="Settings stays pinned at the bottom so it is always easy to find. The rest of this order follows the owner account across browsers." /></div>
          <ol class="navigation-order-list">{navigationOrder.map((section, index) => { const item = navigationLabels[section]; return <li key={section}><span><Icon name={item.icon} size={16} /><strong>{item.label}</strong></span><div><button type="button" disabled={index === 0} onClick={() => onNavigationOrderChange(moveNavigationItem(navigationOrder, section, -1))} aria-label={`Move ${item.label} up`}><Icon name="chevron" size={14} class="icon--up" /></button><button type="button" disabled={index === navigationOrder.length - 1} onClick={() => onNavigationOrderChange(moveNavigationItem(navigationOrder, section, 1))} aria-label={`Move ${item.label} down`}><Icon name="chevron" size={14} class="icon--down" /></button></div></li>; })}</ol>
          <div class="settings-card__foot"><span>{preferenceSyncStatus === 'local' ? 'Browser fallback active' : 'Synced through Helix'}</span><button class="button button--quiet" type="button" onClick={() => onNavigationOrderChange([...primaryDashboardSections])}>Reset order</button></div>
        </section>
        <HostIntegrationSettings resource={hostIntegration} user={user} csrfToken={csrfToken} onRefresh={onHostIntegrationRefresh} />
        <AccountSettings user={user} csrfToken={csrfToken} onAccountUpdated={onAccountUpdated} />
      </div>
    </div>
  );
}
