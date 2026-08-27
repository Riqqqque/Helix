import { describe, expect, it } from 'vitest';
import {
  jobPollFailureDecision,
  MAX_AUTOMATIC_JOB_POLL_FAILURES,
} from './job-polling';

describe('job polling recovery', () => {
  it('backs off transient failures and then pauses at a bounded limit', () => {
    expect(jobPollFailureDecision(0, 1_000)).toEqual({
      consecutiveFailures: 1,
      paused: false,
      retryDelayMs: 1_000,
    });
    expect(jobPollFailureDecision(1, 1_000)).toEqual({
      consecutiveFailures: 2,
      paused: false,
      retryDelayMs: 2_000,
    });
    expect(jobPollFailureDecision(2, 1_000)).toEqual({
      consecutiveFailures: MAX_AUTOMATIC_JOB_POLL_FAILURES,
      paused: true,
      retryDelayMs: null,
    });
  });

  it('caps the automatic retry delay', () => {
    expect(jobPollFailureDecision(1, 3_000).retryDelayMs).toBe(4_000);
  });
});
