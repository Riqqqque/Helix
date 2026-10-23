import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { ConsoleText, consoleLevel, isHelixStatusCheckLine, parseConsoleLine } from './console-format';

describe('console formatting', () => {
  it('reads ANSI colors, resets, and 256-color codes', () => {
    const spans = parseConsoleLine('\u001b[33mWarn\u001b[0m plain \u001b[1;38;5;196mhot\u001b[m');
    expect(spans.map((span) => span.text)).toEqual(['Warn', ' plain ', 'hot']);
    expect(spans[0]?.color).toBe('#facc15');
    expect(spans[1]?.color).toBeNull();
    expect(spans[2]?.bold).toBe(true);
    expect(spans[2]?.color).toBe('rgb(255,0,0)');
  });

  it('reads Minecraft § codes and strips other control sequences', () => {
    const spans = parseConsoleLine('§aGreen §lbold§r normal\u001b]0;title\u0007 \u001b[2Kend');
    expect(spans[0]).toMatchObject({ text: 'Green ', color: '#69db7c', bold: false });
    expect(spans[1]).toMatchObject({ text: 'bold', bold: true });
    expect(spans.map((span) => span.text).join('')).toBe('Green bold normal end');
  });

  it('finds log levels and Helix status-check noise', () => {
    expect(consoleLevel('[12:00:00 WARN]: Can\'t keep up!')).toBe('warn');
    expect(consoleLevel('[12:00:00 ERROR]: Could not load plugin')).toBe('error');
    expect(consoleLevel('\tat com.example.Plugin.onEnable(Plugin.java:10)')).toBe('error');
    expect(consoleLevel('[12:00:00 INFO]: Done (3.2s)!')).toBe('info');
    expect(consoleLevel('plain text')).toBeNull();
    expect(isHelixStatusCheckLine('[05:06:16 INFO]: Thread RCON Client /172.17.0.1 shutting down')).toBe(true);
    expect(isHelixStatusCheckLine('[05:06:16 INFO]: Steve joined the game')).toBe(false);
  });

  it('renders colored spans and level tints', () => {
    const html = render(<ConsoleText text={'\u001b[31mred\u001b[0m\n[12:00:00 WARN]: careful'} />);
    expect(html).toContain('color:#f87171');
    expect(html).toContain('console-level--warn');
    expect(html).not.toContain('\u001b');
  });
});
