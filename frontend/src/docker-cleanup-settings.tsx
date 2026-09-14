import { useCallback, useEffect, useState } from 'preact/hooks';
import type { BrokerJob } from './control-api';
import { CreateJobProgress } from './create-job-progress';
import {
  deleteDockerCleanupSchedule,
  getDockerCleanupJob,
  getDockerCleanupStatus,
  setDockerCleanupSchedule,
  startDockerCleanup,
  type DockerCleanupSchedule,
  type DockerCleanupStatus,
} from './docker-cleanup-api';
import { InlineError } from './dashboard-ui';
import { formatBytes, formatTimestamp } from './format';
import type { RebootWeekday } from './host-api';
import { Icon } from './icons';
import { InfoTip } from './info-tip';
import { useJobPolling } from './job-polling';
import { Dialog } from './modal';

const cleanupWeekdays: readonly { id: RebootWeekday; short: string; label: string }[] = [
  { id: 'monday', short: 'Mon', label: 'Monday' },
  { id: 'tuesday', short: 'Tue', label: 'Tuesday' },
  { id: 'wednesday', short: 'Wed', label: 'Wednesday' },
  { id: 'thursday', short: 'Thu', label: 'Thursday' },
  { id: 'friday', short: 'Fri', label: 'Friday' },
  { id: 'saturday', short: 'Sat', label: 'Saturday' },
  { id: 'sunday', short: 'Sun', label: 'Sunday' },
];

const categoryLabels = {
  images: 'Images',
  containers: 'Containers',
  local_volumes: 'Volumes',
  build_cache: 'Build cache',
} as const;

function initialJob(id: string): BrokerJob {
  const now = Date.now();
  return {
    id,
    kind: 'docker_cleanup',
    status: 'queued',
    stage: 'Queued',
    progressPercent: 0,
    createdAtUnixMs: now,
    updatedAtUnixMs: now,
    result: null,
    error: null,
  };
}

function DockerCleanupScheduleDialog({
  schedule,
  timezone,
  csrfToken,
  onClose,
  onChanged,
}: {
  schedule: DockerCleanupSchedule;
  timezone: string | null;
  csrfToken: string;
  onClose: () => void;
  onChanged: () => Promise<void>;
}) {
  const existing = schedule.state === 'scheduled' || schedule.state === 'degraded' ? schedule : null;
  const [weekdays, setWeekdays] = useState<RebootWeekday[]>(existing?.weekdays ?? cleanupWeekdays.map((day) => day.id));
  const [time, setTime] = useState(existing === null ? '04:30' : `${String(existing.hour).padStart(2, '0')}:${String(existing.minute).padStart(2, '0')}`);
  const [retentionHours, setRetentionHours] = useState(existing?.retentionHours ?? 168);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const toggleDay = (day: RebootWeekday): void => {
    setWeekdays((current) => current.includes(day)
      ? current.filter((entry) => entry !== day)
      : cleanupWeekdays.map((entry) => entry.id).filter((entry) => entry === day || current.includes(entry)));
  };

  const save = async (): Promise<void> => {
    if (busy || timezone === null || weekdays.length === 0 || !/^(?:[01]\d|2[0-3]):[0-5]\d$/u.test(time)) return;
    const hour = Number(time.slice(0, 2));
    const minute = Number(time.slice(3, 5));
    setBusy(true);
    setError(null);
    try {
      await setDockerCleanupSchedule({ weekdays, hour, minute, timezone, retentionHours }, csrfToken);
      await onChanged();
      onClose();
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : 'Helix could not save the Docker cleanup schedule.');
    } finally {
      setBusy(false);
    }
  };

  const remove = async (): Promise<void> => {
    if (busy || existing === null) return;
    setBusy(true);
    setError(null);
    try {
      await deleteDockerCleanupSchedule(csrfToken);
      await onChanged();
      onClose();
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : 'Helix could not remove the Docker cleanup schedule.');
    } finally {
      setBusy(false);
    }
  };

  const invalid = timezone === null || weekdays.length === 0 || !/^(?:[01]\d|2[0-3]):[0-5]\d$/u.test(time);
  return <Dialog title={existing === null ? 'Schedule Docker cleanup' : 'Docker cleanup schedule'} onClose={onClose} wide>
    <div class="docker-cleanup-dialog-intro">
      <Icon name="clock" size={18} />
      <span><strong>Safe cleanup only</strong><small>Scheduled runs preserve every container, volume, named image, and active resource. Missed runs do not catch up after boot.</small></span>
    </div>
    {existing !== null && <div class={`recurring-reboot-current recurring-reboot-current--${existing.state}`}><span><strong>{existing.state === 'scheduled' ? 'Schedule active' : 'Schedule needs attention'}</strong><small>{existing.nextAtUnixMs === null ? 'The next run could not be verified.' : `Next run: ${formatTimestamp(existing.nextAtUnixMs)}`}</small></span><small>{existing.calendarExpression}</small></div>}
    {schedule.state === 'unavailable' && <InlineError message={`The saved schedule cannot be changed safely: ${schedule.reason.replaceAll('_', ' ')}.`} />}
    <fieldset class="recurring-reboot-days" disabled={busy || schedule.state === 'unavailable'}>
      <legend>Days</legend>
      <div>{cleanupWeekdays.map((day) => <button key={day.id} type="button" class={weekdays.includes(day.id) ? 'is-active' : ''} aria-pressed={weekdays.includes(day.id)} title={day.label} onClick={() => toggleDay(day.id)}>{day.short}</button>)}</div>
      <button class="recurring-days-toggle" type="button" onClick={() => setWeekdays(weekdays.length === 7 ? [] : cleanupWeekdays.map((day) => day.id))}>{weekdays.length === 7 ? 'Clear days' : 'Every day'}</button>
    </fieldset>
    <div class="reboot-confirmation-grid docker-cleanup-schedule-fields">
      <label><span>Host time</span><input type="time" value={time} disabled={busy || timezone === null} onInput={(event) => setTime(event.currentTarget.value)} /><small>{timezone === null ? 'Linux timezone is unavailable.' : `Linux timezone: ${timezone}`}</small></label>
      <label><span>Keep recent data</span><select value={retentionHours} disabled={busy} onChange={(event) => setRetentionHours(Number(event.currentTarget.value))}><option value={24}>1 day</option><option value={72}>3 days</option><option value={168}>7 days</option><option value={336}>14 days</option><option value={720}>30 days</option><option value={2160}>90 days</option></select><small>Only older, safely rebuildable data is eligible.</small></label>
    </div>
    <InlineError message={error} />
    <div class="dialog-actions dialog-actions--split">{existing === null ? <span /> : <button class="button button--danger-quiet" type="button" disabled={busy} onClick={() => void remove()}>{busy ? 'Working…' : 'Remove schedule'}</button>}<span><button class="button button--quiet" type="button" disabled={busy} onClick={onClose}>Cancel</button><button class="button button--primary" type="button" disabled={busy || invalid || schedule.state === 'unavailable'} onClick={() => void save()}>{busy ? 'Saving…' : existing === null ? 'Create schedule' : 'Save schedule'}</button></span></div>
  </Dialog>;
}

export function DockerCleanupSettings({ csrfToken, timezone, canManage }: {
  csrfToken: string;
  timezone: string | null;
  canManage: boolean;
}) {
  const [status, setStatus] = useState<DockerCleanupStatus | null>(null);
  const [job, setJob] = useState<BrokerJob | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [scheduleOpen, setScheduleOpen] = useState(false);

  const load = useCallback(async (): Promise<void> => {
    setLoading(true);
    try {
      const next = await getDockerCleanupStatus(csrfToken);
      setStatus(next);
      if (next.activeJob !== null) setJob(next.activeJob);
      setError(null);
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : 'Docker cleanup status could not be loaded.');
    } finally {
      setLoading(false);
    }
  }, [csrfToken]);

  useEffect(() => {
    void load();
  }, [load]);

  const polling = useJobPolling({
    job,
    csrfToken,
    getJob: getDockerCleanupJob,
    onJob: setJob,
    onComplete: async () => {
      setJob(null);
      await load();
    },
    onSessionExpired: () => setError('Your Helix session expired. The cleanup continues safely on the server.'),
  });
  const active = job?.status === 'queued' || job?.status === 'running';

  const run = async (): Promise<void> => {
    if (!canManage || active || status?.availability !== 'ready') return;
    setError(null);
    try {
      const dispatch = await startDockerCleanup(status.defaultRetentionHours, csrfToken);
      setJob(initialJob(dispatch.jobId));
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : 'Docker cleanup could not be started.');
    }
  };

  const schedule = status?.schedule ?? { state: 'none' as const };
  const lastRun = status?.lastRun;
  return <>
    <section class="settings-card settings-card--docker-cleanup">
      <div class="settings-card__head"><div><Icon name="trash" /><span><h2>Docker cleanup</h2><p>Reclaim old build data without touching containers or volumes.</p></span></div><InfoTip text="Helix removes only build cache, dangling images, and unused networks older than the selected age. Running and stopped containers, named images, and every Docker volume are preserved." /></div>
      <InlineError message={error ?? status?.error ?? polling.error} />
      {status === null ? <div class="host-integration-empty"><Icon name="refresh" class={loading ? 'is-spinning' : undefined} /><span>{loading ? 'Measuring Docker storage…' : 'Docker storage is unavailable.'}</span></div> : <>
        <div class="docker-cleanup-summary">
          <div><span>Docker reports reclaimable</span><strong>{formatBytes(status.reclaimableBytes)}</strong><small>Across all unused categories; the safe run may reclaim less.</small></div>
          <div><span>Docker data location</span><strong>{status.storage?.mountPoint ?? 'Unavailable'}</strong><small>{status.storage === null ? 'Docker did not report a data root.' : `${status.storage.dataRoot} · ${status.storage.source}`}</small></div>
          <div><span>Filesystem available</span><strong>{status.storage === null ? '—' : formatBytes(status.storage.availableBytes)}</strong><small>{status.storage === null ? 'Unavailable' : `${formatBytes(status.storage.totalBytes)} total · ${status.storage.storageDriver}`}</small></div>
        </div>
        <div class="docker-usage-grid">{status.usage.map((item) => <div key={item.kind}><span>{categoryLabels[item.kind]}</span><strong>{formatBytes(item.sizeBytes)}</strong><small>{item.totalCount} total · {formatBytes(item.reclaimableBytes)} reclaimable</small></div>)}</div>
        <div class="docker-cleanup-protection"><Icon name="check" size={16} /><span><strong>Containers and volumes stay intact</strong><small>Helix also preserves named images and anything Docker considers active. The schedule cleans the daemon’s configured storage location; changing drives belongs in Docker’s daemon configuration.</small></span></div>
        {job !== null && <div class="docker-cleanup-job"><CreateJobProgress job={job} copy="The cleanup runs on the host. Refreshing or closing this page does not cancel it." />{job.status === 'failed' && <button class="button button--quiet" type="button" onClick={() => { setJob(null); void load(); }}>Dismiss</button>}{polling.paused && <button class="button button--quiet" type="button" onClick={polling.resume}>Resume status</button>}</div>}
        <div class="docker-cleanup-actions"><span>{lastRun === null || lastRun === undefined ? 'No cleanup has been recorded yet.' : lastRun.status === 'unavailable' ? 'Cleanup history needs attention.' : lastRun.status === 'running' ? 'A cleanup was running when status was recorded.' : `${lastRun.trigger === 'scheduled' ? 'Scheduled' : 'Manual'} cleanup ${lastRun.status} ${lastRun.finishedAtUnixMs === null || lastRun.finishedAtUnixMs === undefined ? '' : formatTimestamp(lastRun.finishedAtUnixMs)}.`}</span><div><button class="button button--quiet" type="button" disabled={!canManage || status.availability !== 'ready'} onClick={() => setScheduleOpen(true)}>{schedule.state === 'scheduled' || schedule.state === 'degraded' ? 'Edit schedule' : 'Schedule'}</button><button class="button button--primary" type="button" disabled={!canManage || active || status.availability !== 'ready'} onClick={() => void run()}>{active ? 'Cleaning…' : 'Clean Docker now'}</button></div></div>
        <div class="host-recurring-state"><Icon name="clock" size={15} /><span>{schedule.state === 'none' ? 'No automatic cleanup schedule.' : schedule.state === 'unavailable' ? 'The cleanup schedule cannot be verified safely.' : schedule.nextAtUnixMs === null ? 'Cleanup schedule exists; next run could not be verified.' : `Next safe cleanup: ${formatTimestamp(schedule.nextAtUnixMs)} (${schedule.timezone}); keeps ${schedule.retentionHours / 24} days.`}</span></div>
        {!canManage && <div class="host-integration-notice"><Icon name="info" size={14} />This account can view Docker storage but cannot clean it or change its schedule.</div>}
      </>}
    </section>
    {scheduleOpen && <DockerCleanupScheduleDialog schedule={schedule} timezone={timezone} csrfToken={csrfToken} onClose={() => setScheduleOpen(false)} onChanged={load} />}
  </>;
}
