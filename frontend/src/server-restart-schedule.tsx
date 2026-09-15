import { useEffect, useRef, useState } from 'preact/hooks';
import { ApiError, expectRecord, requestJson } from './api';
import { InlineError } from './dashboard-ui';
import { Icon } from './icons';
import { Dialog } from './modal';

export interface RestartSchedule {
  enabled: boolean;
  state: string;
  intervalHours: number | null;
  nextAt: number | null;
  lastResult: string;
  playerWarnings?: boolean;
  shutdownMethod?: string;
  runtimeUpdateRequired?: boolean;
}

export function parseRestartSchedule(value: unknown): RestartSchedule | null {
  if (value === undefined || value === null) return null;
  const root = expectRecord(value, 'restart schedule');
  const valid = ['disabled', 'scheduled', 'warning', 'restarting', 'unavailable'];
  if (typeof root.state !== 'string' || !valid.includes(root.state) || typeof root.enabled !== 'boolean') throw new Error('Invalid restart schedule status');
  const interval = typeof root.interval_hours === 'number' && [6, 12, 24, 48, 168].includes(root.interval_hours) ? root.interval_hours : null;
  const nextAt = typeof root.next_at_unix_ms === 'number' && Number.isSafeInteger(root.next_at_unix_ms) && root.next_at_unix_ms > 0 ? root.next_at_unix_ms : null;
  if (root.enabled && (interval === null || nextAt === null)) throw new Error('Invalid active restart schedule');
  return { enabled: root.enabled, state: root.state, intervalHours: interval, nextAt, lastResult: typeof root.last_result === 'string' ? root.last_result : '', playerWarnings: root.player_warnings !== false, shutdownMethod: typeof root.shutdown_method === 'string' ? root.shutdown_method : '', runtimeUpdateRequired: root.runtime_update_required === true };
}

export function restartScheduleInput(time: string, hours: number, now = Date.now(), existing?: RestartSchedule | null) {
  if (![6, 12, 24, 48, 168].includes(hours)) throw new Error('Choose a supported interval.');
  if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(time) || !Number.isFinite(now)) throw new Error('Choose a valid restart time.');
  // Leave a minute of request headroom beyond the broker's full warning window.
  const earliest = now + 420_000;
  if (existing?.enabled && existing.intervalHours === hours && existing.nextAt !== null && existing.nextAt >= earliest && clockInput(existing.nextAt) === time) {
    return { first_at_unix_ms: existing.nextAt, interval_hours: hours };
  }
  const [hour = 0, minute = 0] = time.split(':').map(Number);
  const start = new Date(now);
  if (!Number.isFinite(start.getTime())) throw new Error('The current time is unavailable.');
  start.setHours(hour, minute, 0, 0);
  if (hours >= 24) {
    while (start.getTime() < earliest) {
      start.setDate(start.getDate() + 1);
      start.setHours(hour, minute, 0, 0);
    }
  } else {
    while (start.getTime() < earliest) start.setTime(start.getTime() + hours * 3_600_000);
  }
  const timestamp = start.getTime();
  return { first_at_unix_ms: timestamp, interval_hours: hours };
}

function clockInput(timestamp: number): string {
  const date = new Date(timestamp);
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`;
}

function nextLabel(timestamp: number): string {
  return new Date(timestamp).toLocaleString(undefined, { weekday: 'short', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
}

export function ServerRestartSchedulePanel({ id, name, schedule: suppliedSchedule, csrfToken, canManage, onSaved, onSessionExpired }: {
  id: string; name: string; schedule: RestartSchedule | null | undefined; csrfToken: string;
  canManage: boolean; onSaved: () => void; onSessionExpired: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [time, setTime] = useState('05:00');
  const [now, setNow] = useState(Date.now());
  const [hours, setHours] = useState(24);
  const [acknowledged, setAcknowledged] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [remoteSchedule, setRemoteSchedule] = useState<RestartSchedule | null>(null);
  const mutation = useRef(0);
  const saving = useRef(false);
  const schedule = suppliedSchedule === undefined ? remoteSchedule : suppliedSchedule;
  const playerWarnings = schedule?.playerWarnings !== false;
  useEffect(() => {
    if (suppliedSchedule !== undefined) return;
    const controller = new AbortController();
    let loading = false;
    const refresh = async () => {
      if (loading || saving.current) return;
      loading = true;
      const revision = mutation.current;
      try {
        const result = await requestJson(`/api/v1/servers/${encodeURIComponent(id)}/restart-schedule`, parseRestartSchedule, { csrfToken, signal: controller.signal });
        if (!controller.signal.aborted && revision === mutation.current) setRemoteSchedule(result);
      } catch (failure) {
        if (!controller.signal.aborted && revision === mutation.current) {
          setError(failure instanceof Error ? failure.message : 'Schedule status unavailable');
          if (failure instanceof ApiError && failure.status === 401) onSessionExpired();
        }
      } finally { loading = false; }
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 10_000);
    return () => { controller.abort(); window.clearInterval(timer); };
  }, [id, csrfToken, suppliedSchedule, onSessionExpired]);
  const unavailable = schedule === null || schedule === undefined || schedule.state === 'unavailable';
  const executing = schedule?.state === 'restarting';
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  useEffect(() => {
    if (!open) return;
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, [open]);
  const preview = restartScheduleInput(time, hours, now, schedule);
  const [clockHour = 0, clockMinute = 0] = time.split(':').map(Number);
  const setClock = (hour: number, minute: number) => setTime(`${String(hour).padStart(2, '0')}:${String(minute).padStart(2, '0')}`);
  const needsAcknowledgment = !playerWarnings;
  const show = () => {
    setTime(schedule?.nextAt ? clockInput(schedule.nextAt) : '05:00');
    setNow(Date.now());
    setHours(schedule?.intervalHours ?? 24);
    setAcknowledged(false);
    setError(null);
    setOpen(true);
  };
  async function save(disable: boolean) {
    if (!canManage || busy || saving.current || executing || unavailable) return;
    setError(null);
    try {
      if (!disable && needsAcknowledgment && !acknowledged) throw new Error('Confirm restarting without in-game warnings.');
      if (!disable && schedule?.runtimeUpdateRequired) throw new Error('Update this server’s runtime before enabling a schedule.');
      const body = disable ? null : { ...restartScheduleInput(time, hours, Date.now(), schedule), allow_unwarned_restart: needsAcknowledgment && acknowledged };
      setBusy(true);
      saving.current = true;
      mutation.current += 1;
      const result = await requestJson(`/api/v1/servers/${encodeURIComponent(id)}/restart-schedule`, parseRestartSchedule, { method: 'PUT', body, csrfToken });
      setRemoteSchedule(result);
      setOpen(false);
      onSaved();
    } catch (failure) {
      if (failure instanceof ApiError && failure.status === 401) onSessionExpired();
      setError(failure instanceof Error ? failure.message : 'Could not save the restart schedule.');
    } finally { saving.current = false; setBusy(false); }
  }
  return <section class="server-restart-schedule">
    <div class="server-restart-schedule-heading"><h3><Icon name="clock" size={16} /> Scheduled restarts</h3>
      <button class="button button--quiet" type="button" disabled={!canManage || busy || unavailable || executing} onClick={show}><Icon name="settings" size={15} /> {schedule?.enabled ? 'Edit schedule' : 'Set schedule'}</button></div>
    <p>{unavailable ? 'Schedule status unavailable' : executing ? 'Saving and restarting' : schedule?.state === 'warning' ? (playerWarnings ? 'Player warning countdown in progress' : 'Restart countdown in progress') : schedule?.enabled && schedule.nextAt !== null ? `Next restart: ${nextLabel(schedule.nextAt)}` : 'No scheduled restarts'}</p>
    {schedule?.enabled && <p>Every {schedule.intervalHours} hours · {playerWarnings ? '5-minute player warning' : 'No in-game warnings'}</p>}
    {schedule?.shutdownMethod && <details class="server-restart-details"><summary>Shutdown details</summary><p>{schedule.shutdownMethod}</p></details>}
    {schedule?.runtimeUpdateRequired && <p role="alert">Update this server's runtime before enabling a schedule.</p>}
    {schedule?.lastResult && !['Schedule saved', 'Schedule disabled'].includes(schedule.lastResult) && <p role="status">{schedule.lastResult}</p>}
    {!open && <InlineError message={error} />}
    {schedule?.state === 'warning' && <button class="button button--danger-quiet" type="button" disabled={!canManage || busy} onClick={() => void save(true)}>Cancel countdown and disable</button>}
    {open && <Dialog title="Scheduled restarts" onClose={() => { if (!busy) setOpen(false); }}>
      <form class="server-restart-schedule-form" onSubmit={(event) => { event.preventDefault(); void save(false); }}>
        <p class="server-restart-server">{name}</p>
        <div class="server-restart-fields">
          <label>Every<select value={hours} disabled={busy} onChange={(event) => setHours(Number(event.currentTarget.value))}>{[6, 12, 24, 48, 168].map((value) => <option key={value} value={value}>{value === 168 ? '7 days' : `${value} hours`}</option>)}</select></label>
          <fieldset class="server-restart-clock"><legend>Starting at</legend><div>
            <select aria-label="Hour" value={clockHour % 12 || 12} disabled={busy} onChange={(event) => setClock(Number(event.currentTarget.value) % 12 + (clockHour >= 12 ? 12 : 0), clockMinute)}>{Array.from({length:12}, (_, index) => <option value={index + 1} key={index}>{index + 1}</option>)}</select>
            <span aria-hidden="true">:</span>
            <select aria-label="Minute" value={clockMinute} disabled={busy} onChange={(event) => setClock(clockHour, Number(event.currentTarget.value))}>{Array.from({length:60}, (_, index) => <option value={index} key={index}>{String(index).padStart(2, '0')}</option>)}</select>
            <select aria-label="AM or PM" value={clockHour >= 12 ? 'PM' : 'AM'} disabled={busy} onChange={(event) => setClock(clockHour % 12 + (event.currentTarget.value === 'PM' ? 12 : 0), clockMinute)}><option>AM</option><option>PM</option></select>
          </div></fieldset>
        </div>
        <p class="server-restart-timezone">{timezone.replaceAll('_', ' ')}</p>
        <div class="server-restart-preview" aria-live="polite"><Icon name="clock" size={20} /><div><span>Next restart</span><strong>{nextLabel(preview.first_at_unix_ms)}</strong><span>Then every {hours === 168 ? '7 days' : `${hours} hours`}</span></div></div>
        <p class="server-restart-safety">{playerWarnings ? 'Players online will receive a 5-minute warning before disconnecting.' : 'No in-game warnings are available for this server.'}</p>
        {needsAcknowledgment && <label class="server-restart-ack"><input type="checkbox" checked={acknowledged} disabled={busy || schedule?.runtimeUpdateRequired} onChange={(event) => setAcknowledged(event.currentTarget.checked)} /> Allow restarts without in-game warnings.</label>}
        <details class="server-restart-details"><summary>Timing and safeguards</summary><p>Passed times or times too close for warnings move to the next safe occurrence. Stopped servers and missed runs stay skipped. Intervals use elapsed hours, so local times may shift at daylight saving changes.</p></details>
        {schedule?.runtimeUpdateRequired && <InlineError message="Update this server’s runtime before enabling a schedule." />}
        <InlineError message={error} />
        <div class="server-restart-actions">{schedule?.enabled && <button class="button button--danger-quiet" type="button" disabled={busy} onClick={() => void save(true)}>Disable</button>}<button class="button button--quiet" type="button" disabled={busy} onClick={() => setOpen(false)}>Cancel</button><button class="button" type="submit" disabled={busy || executing || unavailable || schedule?.runtimeUpdateRequired || (needsAcknowledgment && !acknowledged)}>{busy ? 'Saving…' : 'Save schedule'}</button></div>
      </form>
    </Dialog>}
  </section>;
}
