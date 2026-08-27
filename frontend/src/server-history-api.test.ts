import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  getServerLogHistory,
  mergeLatestConsoleHistory,
  parseConsoleHistoryPage,
  prependOlderConsoleHistory,
  type ConsoleHistoryEntry,
} from './server-history-api';

const output = (text: string, timestamp: string | null = null): ConsoleHistoryEntry => ({
  kind: 'output', text, timestamp, timestampUnixMs: null, bootStartedAt: null,
});

const page = {
  schema_version: 1,
  instance_id: 'helix:5aac0c6d-f2d2-416c-8ab8-917c9a4aeb7c',
  entries: [
    { kind: 'boot', text: '[helix 1800000000000] ---- Minecraft boot 2027-01-15T08:00:00Z (server) ----', timestamp: null, timestamp_unix_ms: 1_800_000_000_000, boot_started_at: '2027-01-15T08:00:00Z' },
    { kind: 'command', text: '[helix 1800000001000] > /list', timestamp: null, timestamp_unix_ms: 1_800_000_001_000, boot_started_at: null },
    { kind: 'command_response', text: '[helix 1800000001100] < There are 0 players online', timestamp: null, timestamp_unix_ms: 1_800_000_001_100, boot_started_at: null },
  ],
  next_cursor: 'h1.000000000000000a.0000000000000040',
  has_more: true,
  order: 'chronological_within_page',
  pagination_direction: 'newest_to_older',
  page_text_byte_limit: 4 * 1024 * 1024,
  retention: { maximum_bytes: 64 * 1024 * 1024, files: 8, scope: 'per_server' },
  collected_at_unix_ms: 1_800_000_002_000,
};

afterEach(() => vi.unstubAllGlobals());

describe('persistent server console history', () => {
  it('parses markers, paging direction, and per-server retention', () => {
    expect(parseConsoleHistoryPage(page)).toMatchObject({
      instanceId: page.instance_id,
      entries: [{ kind: 'boot', bootStartedAt: '2027-01-15T08:00:00Z' }, { kind: 'command' }, { kind: 'command_response' }],
      nextCursor: page.next_cursor,
      hasMore: true,
      retention: { maximumBytes: 64 * 1024 * 1024, files: 8, scope: 'per_server' },
    });
  });

  it('rejects inconsistent cursors and pagination flags', () => {
    expect(() => parseConsoleHistoryPage({ ...page, next_cursor: null })).toThrow(/pagination/i);
    expect(() => parseConsoleHistoryPage({ ...page, next_cursor: 'unsafe/path' })).toThrow(/cursor/i);
  });

  it('encodes opaque instance IDs and cursors in the history request', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify(page), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    }));
    vi.stubGlobal('fetch', fetchMock);
    await getServerLogHistory('helix:id / unusual', 'csrf-token', page.next_cursor, 100);
    const [path, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(path).toBe(`/api/v1/servers/helix%3Aid%20%2F%20unusual/logs/history?lines=100&cursor=${encodeURIComponent(page.next_cursor)}`);
    expect(request.headers).toMatchObject({ 'X-Helix-CSRF': 'csrf-token' });
  });

  it('appends only the new tail when live pages overlap', () => {
    const first = output('first', '2027-01-15T08:00:00Z');
    const second = output('second', '2027-01-15T08:00:01Z');
    const third = output('third', '2027-01-15T08:00:02Z');
    expect(mergeLatestConsoleHistory([first, second], [second, third])).toEqual([first, second, third]);
  });

  it('preserves repeated identical entries within an incoming page', () => {
    const repeat = output('same line');
    expect(mergeLatestConsoleHistory([], [repeat, repeat, repeat])).toHaveLength(3);
    expect(mergeLatestConsoleHistory([output('before'), repeat], [repeat, repeat])).toEqual([output('before'), repeat, repeat]);
  });

  it('prepends older pages in chronological order without duplicating an overlapping boundary', () => {
    const first = output('first');
    const second = output('second');
    const third = output('third');
    expect(prependOlderConsoleHistory([second, third], [first, second])).toEqual([first, second, third]);
  });
});
