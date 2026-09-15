import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { parseRestartSchedule, restartScheduleInput, ServerRestartSchedulePanel } from './server-restart-schedule';

describe('scheduled server restarts', () => {
  it('shows manager-specific shutdown behavior without promising unavailable warnings', () => {
    const schedule = parseRestartSchedule({enabled:true,state:'scheduled',interval_hours:24,next_at_unix_ms:Date.now()+3_600_000,player_warnings:false,shutdown_method:'AMP application Stop / Start',runtime_update_required:false});
    const html = render(<ServerRestartSchedulePanel id="amp:test" name="Test" schedule={schedule} csrfToken="test" canManage onSaved={() => {}} onSessionExpired={() => {}} />);
    expect(html).toContain('No in-game warnings');
    expect(html).toContain('AMP application Stop / Start');
    expect(html).not.toContain('5-minute player warning');
  });
  it('keeps old brokers unavailable rather than inventing an enabled schedule', () => {
    expect(parseRestartSchedule(undefined)).toBeNull();
    expect(() => parseRestartSchedule({ enabled: true, state: 'scheduled' })).toThrow();
    expect(() => parseRestartSchedule({ enabled: false, state: 'anything' })).toThrow();
  });
  it('takes a clock time without requiring a calendar date', () => {
    const now = new Date(2026, 8, 9, 10, 0).getTime();
    const result = restartScheduleInput('15:30', 24, now);
    expect(result).toEqual({first_at_unix_ms:new Date(2026, 8, 9, 15, 30).getTime(),interval_hours:24});
    for (const time of ['bad', '', '24:00', '12:60', '5:00', '2026-09-09T15:30']) expect(() => restartScheduleInput(time, 24, now)).toThrow();
    expect(() => restartScheduleInput('05:00', 0, now)).toThrow();
  });
  it('automatically rolls passed and too-close times forward', () => {
    const now = new Date(2026, 8, 9, 4, 58).getTime();
    expect(restartScheduleInput('05:00', 24, now).first_at_unix_ms).toBe(new Date(2026, 8, 10, 5).getTime());
    expect(restartScheduleInput('04:00', 24, now).first_at_unix_ms).toBe(new Date(2026, 8, 10, 4).getTime());
    expect(restartScheduleInput('05:00', 6, now).first_at_unix_ms).toBe(new Date(2026, 8, 9, 11).getTime());
    expect(restartScheduleInput('05:00', 12, now).first_at_unix_ms).toBe(new Date(2026, 8, 9, 17).getTime());
  });
  it('starts longer intervals at the next clock occurrence and preserves existing anchors', () => {
    const now = new Date(2026, 8, 9, 10).getTime();
    for (const hours of [48,168]) {
      expect(restartScheduleInput('05:00', hours, now).first_at_unix_ms).toBe(new Date(2026, 8, 10, 5).getTime());
      const existing = {enabled:true,state:'scheduled',intervalHours:hours,nextAt:new Date(2026, 8, 13, 5).getTime(),lastResult:''};
      expect(restartScheduleInput('05:00', hours, now, existing).first_at_unix_ms).toBe(existing.nextAt);
      expect(restartScheduleInput('06:00', hours, now, existing).first_at_unix_ms).toBe(new Date(2026, 8, 10, 6).getTime());
    }
  });
  it('handles midnight, noon and year rollover without date boundaries', () => {
    const now = new Date(2026, 11, 31, 23, 58).getTime();
    expect(restartScheduleInput('00:00', 24, now).first_at_unix_ms).toBe(new Date(2027, 0, 2, 0).getTime());
    expect(restartScheduleInput('12:00', 24, now).first_at_unix_ms).toBe(new Date(2027, 0, 1, 12).getTime());
  });
  it('retains the selected clock time when advancing beyond a daylight-saving transition', () => {
    const now = new Date(2026, 2, 8, 4).getTime();
    expect(restartScheduleInput('02:30', 24, now).first_at_unix_ms).toBe(new Date(2026, 2, 9, 2, 30).getTime());
    expect(() => restartScheduleInput('05:00', 24, Number.MAX_VALUE)).toThrow();
  });
  it('shows next run and last outcome, and protects read-only controls', () => {
    const schedule = parseRestartSchedule({ enabled: true, state: 'scheduled', interval_hours: 24, next_at_unix_ms: Date.now() + 3_600_000, last_result: 'Skipped: server was not running' });
    const html = render(<ServerRestartSchedulePanel id="helix:test" name="Test" schedule={schedule} csrfToken="test" canManage={false} onSaved={() => {}} onSessionExpired={() => {}} />);
    expect(html).toContain('Next restart:');
    expect(html).toContain('Skipped: server was not running');
    expect(html).toMatch(/disabled[^>]*>/);
  });
  it('offers cancellation during warnings but not after shutdown begins', () => {
    const base = { enabled: true, intervalHours: 24, nextAt: Date.now(), lastResult: '' };
    const props = { id: 'helix:test', name: 'Test', csrfToken: 'test', canManage: true, onSaved: () => {}, onSessionExpired: () => {} };
    expect(render(<ServerRestartSchedulePanel {...props} schedule={{ ...base, state: 'warning' }} />)).toContain('Cancel countdown and disable');
    const executing = render(<ServerRestartSchedulePanel {...props} schedule={{ ...base, state: 'restarting' }} />);
    expect(executing).toContain('Saving and restarting');
    expect(executing).not.toContain('Cancel countdown and disable');
  });
});
