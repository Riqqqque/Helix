import { useEffect, useRef, useState } from 'preact/hooks';
import { ApiError } from './api';
import { getJob, type BrokerJob } from './control-api';

export const MAX_AUTOMATIC_JOB_POLL_FAILURES = 3;

export interface JobPollFailureDecision {
  consecutiveFailures: number;
  paused: boolean;
  retryDelayMs: number | null;
}

export function jobPollFailureDecision(
  previousFailures: number,
  baseDelayMs: number,
): JobPollFailureDecision {
  const consecutiveFailures = Math.min(
    MAX_AUTOMATIC_JOB_POLL_FAILURES,
    Math.max(0, previousFailures) + 1,
  );
  const paused = consecutiveFailures >= MAX_AUTOMATIC_JOB_POLL_FAILURES;
  return {
    consecutiveFailures,
    paused,
    retryDelayMs: paused
      ? null
      : Math.min(4_000, baseDelayMs * (2 ** Math.max(0, consecutiveFailures - 1))),
  };
}

function isSessionError(error: unknown): boolean {
  return error instanceof ApiError && (error.status === 401 || error.code === 'csrf_rejected');
}

interface JobPollingOptions {
  job: BrokerJob | null;
  csrfToken: string;
  baseDelayMs?: number;
  onJob: (job: BrokerJob) => void;
  onComplete: () => void | Promise<void>;
  onSessionExpired: () => void;
}

export interface JobPollingController {
  error: string | null;
  paused: boolean;
  resume: () => void;
}

export function useJobPolling({
  job,
  csrfToken,
  baseDelayMs = 1_000,
  onJob,
  onComplete,
  onSessionExpired,
}: JobPollingOptions): JobPollingController {
  const [consecutiveFailures, setConsecutiveFailures] = useState(0);
  const [paused, setPaused] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const callbacks = useRef({ onJob, onComplete, onSessionExpired });
  callbacks.current = { onJob, onComplete, onSessionExpired };

  useEffect(() => {
    if (job === null || (job.status !== 'queued' && job.status !== 'running') || paused) return;
    const delayMs = consecutiveFailures === 0
      ? baseDelayMs
      : Math.min(4_000, baseDelayMs * (2 ** Math.max(0, consecutiveFailures - 1)));
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      void getJob(job.id, csrfToken, controller.signal).then((next) => {
        if (controller.signal.aborted) return;
        setConsecutiveFailures(0);
        setError(null);
        callbacks.current.onJob(next);
        if (next.status === 'complete') {
          void Promise.resolve().then(() => callbacks.current.onComplete()).catch((completionError: unknown) => {
            if (isSessionError(completionError)) callbacks.current.onSessionExpired();
            else setError(completionError instanceof Error ? completionError.message : 'The action finished, but Helix could not refresh the page.');
          });
        }
      }).catch((requestError: unknown) => {
        if (controller.signal.aborted) return;
        if (isSessionError(requestError)) {
          setPaused(true);
          setError('Your Helix session expired while checking this job. The server-side operation was not cancelled.');
          callbacks.current.onSessionExpired();
          return;
        }
        const decision = jobPollFailureDecision(consecutiveFailures, baseDelayMs);
        setConsecutiveFailures(decision.consecutiveFailures);
        setPaused(decision.paused);
        const message = requestError instanceof Error ? requestError.message : 'Helix could not check the job status.';
        setError(decision.paused
          ? `${message} The operation is still running in the background; resume the status check or close this dialog safely.`
          : `${message} Retrying the status check automatically…`);
      });
    }, delayMs);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [baseDelayMs, consecutiveFailures, csrfToken, job, paused]);

  const resume = (): void => {
    setConsecutiveFailures(0);
    setError(null);
    setPaused(false);
  };

  return { error, paused, resume };
}
