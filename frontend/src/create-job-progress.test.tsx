import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import type { BrokerJob } from './control-api';
import { CreateJobProgress, createJobTimingLabel, steamCreateJobCopy } from './create-job-progress';

function job(overrides: Partial<BrokerJob>): BrokerJob {
  return {
    id: 'job-1',
    kind: 'vrising_create',
    status: 'running',
    stage: 'Downloading V Rising and waiting for first boot',
    progressPercent: 67,
    createdAtUnixMs: 1_000,
    updatedAtUnixMs: 126_000,
    result: null,
    error: null,
    ...overrides,
  };
}

describe('create job progress', () => {
  it('shows a spinner, percent, and elapsed time while a create job is running', () => {
    const markup = render(
      <CreateJobProgress job={job({})} copy={steamCreateJobCopy('V Rising', job({}))} />,
    );

    expect(markup).toContain('job-spinner');
    expect(markup).toContain('67%');
    expect(markup).toContain('so far');
    expect(markup).toContain('10–30 minutes');
    expect(markup).toContain('Downloading V Rising and waiting for first boot');
  });

  it('labels a finished job with the recorded elapsed time', () => {
    const finished = job({
      status: 'complete',
      stage: 'Online',
      progressPercent: 100,
    });
    const markup = render(
      <CreateJobProgress job={finished} copy={steamCreateJobCopy('V Rising', finished)} />,
    );

    expect(markup).not.toContain('job-spinner');
    expect(markup).toContain('100%');
    expect(markup).toContain('Finished in 2m 5s');
    expect(markup).toContain('V Rising is online.');
    expect(createJobTimingLabel(finished, 999_999)).toBe('Finished in 2m 5s');
  });
});
