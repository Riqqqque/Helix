import type { ComponentChildren } from 'preact';

/** One run of console text with the styling that applies to it. */
export interface ConsoleSpan {
  text: string;
  color: string | null;
  background: string | null;
  bold: boolean;
  dim: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
}

// Terminal palette tuned for the dark console background.
const ANSI_COLORS = [
  '#4b5563', '#f87171', '#4ade80', '#facc15', '#60a5fa', '#c084fc', '#22d3ee', '#d1d5db',
  '#6b7280', '#fca5a5', '#86efac', '#fde047', '#93c5fd', '#d8b4fe', '#67e8f9', '#f9fafb',
];

// Minecraft formatting codes (§0-§f).
const MINECRAFT_COLORS: Record<string, string> = {
  '0': '#4b5563', '1': '#3b5bdb', '2': '#2f9e44', '3': '#22b8cf', '4': '#c92a2a', '5': '#ae3ec9',
  '6': '#f59f00', '7': '#adb5bd', '8': '#868e96', '9': '#748ffc', a: '#69db7c', b: '#66d9e8',
  c: '#ff6b6b', d: '#f783ac', e: '#ffe066', f: '#f8f9fa',
};

function ansi(index: number): string {
  return ANSI_COLORS[index] ?? '#d1d5db';
}

function blank(): Omit<ConsoleSpan, 'text'> {
  return { color: null, background: null, bold: false, dim: false, italic: false, underline: false, strike: false };
}

function xterm256(index: number): string {
  if (index < 16) return ansi(index);
  if (index >= 232) {
    const level = 8 + (index - 232) * 10;
    return `rgb(${level},${level},${level})`;
  }
  const value = index - 16;
  const scale = [0, 95, 135, 175, 215, 255];
  const step = (position: number): number => scale[position] ?? 0;
  return `rgb(${step(Math.floor(value / 36))},${step(Math.floor(value / 6) % 6)},${step(value % 6)})`;
}

function applySgr(style: Omit<ConsoleSpan, 'text'>, codes: number[]): Omit<ConsoleSpan, 'text'> {
  let next = { ...style };
  for (let index = 0; index < codes.length; index += 1) {
    const code = codes[index] ?? 0;
    if (code === 0) next = blank();
    else if (code === 1) next.bold = true;
    else if (code === 2) next.dim = true;
    else if (code === 3) next.italic = true;
    else if (code === 4) next.underline = true;
    else if (code === 9) next.strike = true;
    else if (code === 22) { next.bold = false; next.dim = false; }
    else if (code === 23) next.italic = false;
    else if (code === 24) next.underline = false;
    else if (code === 29) next.strike = false;
    else if (code >= 30 && code <= 37) next.color = ansi(code - 30);
    else if (code >= 90 && code <= 97) next.color = ansi(code - 90 + 8);
    else if (code === 39) next.color = null;
    else if (code >= 40 && code <= 47) next.background = ansi(code - 40);
    else if (code >= 100 && code <= 107) next.background = ansi(code - 100 + 8);
    else if (code === 49) next.background = null;
    else if ((code === 38 || code === 48) && codes[index + 1] === 5 && codes[index + 2] !== undefined) {
      const color = xterm256(Math.max(0, Math.min(255, codes[index + 2] ?? 0)));
      if (code === 38) next.color = color; else next.background = color;
      index += 2;
    } else if ((code === 38 || code === 48) && codes[index + 1] === 2 && codes[index + 4] !== undefined) {
      const [r = 0, g = 0, b = 0] = codes.slice(index + 2, index + 5).map((part) => Math.max(0, Math.min(255, part)));
      const color = `rgb(${r},${g},${b})`;
      if (code === 38) next.color = color; else next.background = color;
      index += 4;
    }
  }
  return next;
}

/**
 * Splits a console line into styled spans. Understands ANSI SGR colors
 * (including 256-color and true color), Minecraft § codes, and strips other
 * terminal control sequences.
 */
export function parseConsoleLine(line: string): ConsoleSpan[] {
  const spans: ConsoleSpan[] = [];
  let style = blank();
  let buffer = '';
  const flush = (): void => {
    if (buffer.length === 0) return;
    const last = spans[spans.length - 1];
    if (last !== undefined && JSON.stringify({ ...last, text: '' }) === JSON.stringify({ ...style, text: '' })) {
      last.text += buffer;
    } else {
      spans.push({ text: buffer, ...style });
    }
    buffer = '';
  };
  for (let index = 0; index < line.length; index += 1) {
    const character = line.charAt(index);
    if (character === '\u001b') {
      const next = line[index + 1];
      if (next === '[') {
        let end = index + 2;
        while (end < line.length && !/[@-~]/u.test(line.charAt(end))) end += 1;
        if (line[end] === 'm') {
          flush();
          const params = line.slice(index + 2, end);
          const codes = params.length === 0 ? [0] : params.split(';').map((part) => Number.parseInt(part, 10) || 0);
          style = applySgr(style, codes);
        }
        index = end;
        continue;
      }
      if (next === ']') {
        let end = index + 2;
        while (end < line.length && line[end] !== '\u0007' && !(line[end] === '\u001b' && line[end + 1] === '\\')) end += 1;
        index = line[end] === '\u001b' ? end + 1 : end;
        continue;
      }
      index += 1;
      continue;
    }
    if (character === '§' && index + 1 < line.length) {
      const code = line.charAt(index + 1).toLowerCase();
      flush();
      const minecraft = MINECRAFT_COLORS[code];
      if (minecraft !== undefined) style = { ...blank(), color: minecraft };
      else if (code === 'l') style = { ...style, bold: true };
      else if (code === 'o') style = { ...style, italic: true };
      else if (code === 'n') style = { ...style, underline: true };
      else if (code === 'm') style = { ...style, strike: true };
      else if (code === 'r') style = blank();
      index += 1;
      continue;
    }
    if (character < ' ' && character !== '\t') continue;
    buffer += character;
  }
  flush();
  return spans;
}

export type ConsoleLevel = 'error' | 'warn' | 'info' | 'debug' | null;

/** The log level of a line, from markers such as `[12:00:00 WARN]` or `/ERROR]`. */
export function consoleLevel(line: string): ConsoleLevel {
  const head = line.slice(0, 160);
  if (/\b(ERROR|FATAL|SEVERE)\b|^\s*(Caused by:|Exception in thread)|^\s+at [\w$.]+\(/u.test(head)) return 'error';
  if (/\bWARN(ING)?\b/u.test(head)) return 'warn';
  if (/\b(DEBUG|TRACE)\b/u.test(head)) return 'debug';
  if (/\bINFO\b/u.test(head)) return 'info';
  return null;
}

/** Helix's own status checks connect over RCON once a minute; these lines are noise. */
export function isHelixStatusCheckLine(line: string): boolean {
  return /Thread RCON Client \/(127\.0\.0\.1|172\.\d+\.\d+\.\d+) (started|shutting down)/u.test(line);
}

function spanStyle(span: ConsoleSpan): Record<string, string> | undefined {
  const style: Record<string, string> = {};
  if (span.color !== null) style.color = span.color;
  if (span.background !== null) style.backgroundColor = span.background;
  if (span.bold) style.fontWeight = '700';
  if (span.dim) style.opacity = '0.7';
  if (span.italic) style.fontStyle = 'italic';
  const decorations = [span.underline ? 'underline' : '', span.strike ? 'line-through' : ''].filter(Boolean);
  if (decorations.length > 0) style.textDecoration = decorations.join(' ');
  return Object.keys(style).length > 0 ? style : undefined;
}

/** Renders a console line with its colors; plain lines are tinted by log level. */
export function ConsoleText({ text }: { text: string }) {
  const lines = text.split('\n');
  const children: ComponentChildren[] = [];
  lines.forEach((line, lineIndex) => {
    if (lineIndex > 0) children.push('\n');
    const spans = parseConsoleLine(line);
    const colored = spans.some((span) => span.color !== null || span.background !== null);
    const level = colored ? null : consoleLevel(spans.map((span) => span.text).join(''));
    children.push(
      <span key={lineIndex} class={level === null ? undefined : `console-level console-level--${level}`}>
        {spans.map((span, index) => {
          const style = spanStyle(span);
          return style === undefined ? span.text : <span key={index} style={style}>{span.text}</span>;
        })}
      </span>,
    );
  });
  return <>{children}</>;
}
