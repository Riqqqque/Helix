import type { ComponentChildren } from 'preact';
import { useEffect, useState } from 'preact/hooks';
import type { BrokerJob } from './control-api';
import { ProgressBar } from './dashboard-ui';
import { formatDuration } from './format';
import { Icon } from './icons';
import { OperationError } from './operation-error';

function jobElapsedSeconds(job: BrokerJob, now: number): number {
  const end = job.status === 'queued' || job.status === 'running' ? now : job.updatedAtUnixMs;
  return Math.max(0, Math.floor((end - job.createdAtUnixMs) / 1_000));
}

export function createJobTimingLabel(job: BrokerJob, now: number): string {
  const elapsed = formatDuration(jobElapsedSeconds(job, now));
  if (job.status === 'queued' || job.status === 'running') return `${elapsed} so far`;
  if (job.status === 'complete') return `Finished in ${elapsed}`;
  return elapsed;
}

export function CreateJobProgress({
  job,
  copy,
  children,
}: {
  job: BrokerJob;
  copy: string;
  children?: ComponentChildren;
}) {
  const [now, setNow] = useState(() => Date.now());
  const active = job.status === 'queued' || job.status === 'running';
  const percent = Math.max(0, Math.min(100, job.progressPercent));

  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [active, job.id]);

  return (
    <div class="job-progress" role="status" aria-live="polite">
      <div class={`job-icon job-icon--${job.status}${active ? ' job-icon--busy' : ''}`}>
        {active ? (
          <span class="job-spinner" aria-hidden="true" />
        ) : (
          <Icon name={job.status === 'failed' ? 'warning' : 'check'} size={26} />
        )}
      </div>
      <strong>{job.stage}</strong>
      {job.status === 'failed' && job.error ? <OperationError message={job.error} /> : <span>{copy}</span>}
      <ProgressBar
        value={active ? Math.max(percent, 6) : percent}
        tone={job.status === 'failed' ? 'danger' : 'normal'}
      />
      <div class="job-progress-meta">
        <b>{percent}%</b>
        <small>{createJobTimingLabel(job, now)}</small>
      </div>
      {children}
    </div>
  );
}

export function steamCreateJobCopy(game: string, job: BrokerJob): string {
  if (job.status === 'queued' || job.status === 'running') {
    return `Helix is installing ${game} in an isolated container and waiting until it answers. The first download often takes 10–30 minutes. Later creates reuse that runtime and finish faster.`;
  }
  if (job.status === 'complete') return `${game} is online.`;
  return job.error ?? 'Helix stopped this create and rolled back the incomplete server.';
}

export function migrateCreateJobCopy(game: string, job: BrokerJob): string {
  if (job.status === 'queued' || job.status === 'running') {
    return `Helix is creating a new ${game} server and copying the world, plugins, mods, or saves into it. The source AMP or Pterodactyl files stay untouched. First Steam installs can take 10–30 minutes.`;
  }
  if (job.status === 'complete') return `${game} is online on a new Helix server. The old manager still has the original files.`;
  return job.error ?? 'Helix stopped this copy and rolled back the incomplete server. The source was not changed.';
}
