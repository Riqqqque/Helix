import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  deleteDockerCleanupSchedule,
  parseDockerCleanupStatus,
  setDockerCleanupSchedule,
  startDockerCleanup,
} from './docker-cleanup-api';

const runId = 'f52d0674-f617-4c5f-9e04-60e89d6d305f';
const cleanupStatus = {
  schema_version: 1,
  availability: 'ready',
  docker_installed: true,
  storage: {
    data_root: '/var/lib/docker',
    storage_driver: 'overlay2',
    mount_point: '/',
    source: '/dev/mapper/ubuntu-root',
    total_bytes: 500_000_000_000,
    available_bytes: 125_000_000_000,
  },
  usage: [
    { kind: 'images', total_count: 20, active_count: 15, size_bytes: 10_000_000_000, reclaimable_bytes: 1_000_000_000 },
    { kind: 'containers', total_count: 15, active_count: 12, size_bytes: 400_000_000, reclaimable_bytes: 64 },
    { kind: 'local_volumes', total_count: 8, active_count: 8, size_bytes: 800_000_000, reclaimable_bytes: 0 },
    { kind: 'build_cache', total_count: 100, active_count: 0, size_bytes: 5_000_000_000, reclaimable_bytes: 4_000_000_000 },
  ],
  reclaimable_bytes: 5_000_000_064,
  policy: { default_retention_hours: 168, minimum_retention_hours: 24, maximum_retention_hours: 8_760 },
  schedule: { state: 'none' },
  last_run: null,
  active_job: null,
  error: null,
  collected_at_unix_ms: 1_800_000_000_000,
};

afterEach(() => vi.unstubAllGlobals());

describe('Docker cleanup API', () => {
  it('parses storage, usage, policy, and refresh-safe job state', () => {
    expect(parseDockerCleanupStatus({
      ...cleanupStatus,
      active_job: {
        id: runId,
        kind: 'docker_cleanup',
        status: 'running',
        stage: 'Removing old build cache',
        progress_percent: 40,
        created_at_unix_ms: 1_800_000_000_000,
        updated_at_unix_ms: 1_800_000_000_100,
        result: null,
        error: null,
      },
    })).toMatchObject({
      availability: 'ready',
      storage: { dataRoot: '/var/lib/docker', mountPoint: '/' },
      usage: [{ kind: 'images' }, { kind: 'containers' }, { kind: 'local_volumes' }, { kind: 'build_cache' }],
      activeJob: { id: runId, status: 'running' },
      defaultRetentionHours: 168,
    });
  });

  it('rejects unknown categories and unbounded retention values', () => {
    expect(() => parseDockerCleanupStatus({
      ...cleanupStatus,
      usage: [{ kind: 'everything', total_count: 1, active_count: 0, size_bytes: 1, reclaimable_bytes: 1 }],
    })).toThrow(/unknown usage category/i);
    expect(() => parseDockerCleanupStatus({
      ...cleanupStatus,
      policy: { default_retention_hours: 0, minimum_retention_hours: 0, maximum_retention_hours: 99_999 },
    })).toThrow(/default_retention_hours/i);
  });

  it('uses typed mutation routes with CSRF and no destructive scope options', async () => {
    const responses = [
      { job_id: runId, reused: false },
      {
        state: 'scheduled', schedule_id: runId, weekdays: ['monday'], hour: 4, minute: 30,
        timezone: 'America/Denver', retention_hours: 168,
        calendar_expression: 'Mon *-*-* 04:30:00 America/Denver', next_at_unix_ms: null,
        timer_active: true, timer_enabled: true,
      },
      null,
    ];
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(new Response(JSON.stringify(responses.shift()), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    })));
    vi.stubGlobal('fetch', fetchMock);

    await startDockerCleanup(168, 'csrf');
    await setDockerCleanupSchedule({ weekdays: ['monday'], hour: 4, minute: 30, timezone: 'America/Denver', retentionHours: 168 }, 'csrf');
    await deleteDockerCleanupSchedule('csrf');

    const runBody = JSON.parse(String(fetchMock.mock.calls[0]?.[1].body)) as Record<string, unknown>;
    expect(runBody).toEqual({ retention_hours: 168 });
    expect(runBody).not.toHaveProperty('volumes');
    expect(runBody).not.toHaveProperty('containers');
    expect(fetchMock.mock.calls.map((call) => [call[0], call[1].method])).toEqual([
      ['/api/v1/docker/cleanup/run', 'POST'],
      ['/api/v1/docker/cleanup/schedule', 'PUT'],
      ['/api/v1/docker/cleanup/schedule', 'DELETE'],
    ]);
    expect(fetchMock.mock.calls.every((call) => call[1].headers['X-Helix-CSRF'] === 'csrf')).toBe(true);
  });
});
