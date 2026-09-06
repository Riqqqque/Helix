import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { Terminal, type ITheme } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { useEffect, useRef, useState } from 'preact/hooks';
import { ApiError } from './api';
import { DASHBOARD_PREFERENCES_EVENT } from './dashboard-preferences';
import { InlineError, PageHead } from './dashboard-ui';
import { Icon } from './icons';
import { InfoTip } from './info-tip';
import {
  getTerminalStatus,
  requestTerminalTicket,
  terminalWebSocketUrl,
  type TerminalStatus,
} from './terminal-api';
import {
  TERMINAL_FONT_SIZE_DEFAULT,
  encodeBinaryPtyPayload,
  openSafeHttpUrl,
  readClipboardText,
  readStoredTerminalFontSize,
  saveTerminalFontSize,
  safeTerminalTitle,
  terminalKeyAction,
  writeClipboardText,
} from './terminal-keys';
import './terminal.css';

export interface TerminalPageProps {
  csrfToken: string;
  canOpen: boolean;
  onSessionExpired: () => void;
}

type TerminalPhase =
  | 'loading'
  | 'locked'
  | 'authorizing'
  | 'connecting'
  | 'connected'
  | 'ended'
  | 'unavailable';

interface TerminalRuntime {
  terminal: Terminal;
  fit: FitAddon;
  search: SearchAddon;
  socket: WebSocket | null;
  resizeObserver: ResizeObserver | null;
  dataDisposable: { dispose: () => void } | null;
  binaryDisposable: { dispose: () => void } | null;
  resizeDisposable: { dispose: () => void } | null;
  keepalive: number | null;
  pendingOutputBytes: number;
}

interface TerminalChrome {
  copy: () => void;
  paste: () => void;
  openFind: () => void;
  runFind: (direction: 'next' | 'previous', incremental?: boolean) => void;
  closeFind: () => void;
  findOpen: boolean;
  adjustFont: (delta: 1 | -1 | 0) => void;
}

const MAX_PENDING_TERMINAL_OUTPUT_BYTES = 4 * 1024 * 1024;

export function nextTerminalOutputBacklog(pendingBytes: number, incomingBytes: number): number | null {
  if (!Number.isSafeInteger(pendingBytes) || !Number.isSafeInteger(incomingBytes) || pendingBytes < 0 || incomingBytes < 0) return null;
  const next = pendingBytes + incomingBytes;
  return next <= MAX_PENDING_TERMINAL_OUTPUT_BYTES ? next : null;
}

type HostTerminalEvent =
  | { type: 'ready'; user: string; shell: string }
  | { type: 'heartbeat' }
  | { type: 'exit'; exitCode: number; signal: string | null }
  | { type: 'error'; message: string };

function safeEventText(value: unknown, maximum: number): string | null {
  if (
    typeof value !== 'string' ||
    value.length === 0 ||
    value.length > maximum ||
    Array.from(value).some((character) => /\p{Cc}/u.test(character))
  ) return null;
  return value;
}

export function parseHostTerminalEvent(value: string): HostTerminalEvent | null {
  if (value.length > 2_048) return null;
  try {
    const parsed: unknown = JSON.parse(value);
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return null;
    const event = parsed as Record<string, unknown>;
    if (event.type === 'heartbeat' && Object.keys(event).length === 1) return { type: 'heartbeat' };
    if (event.type === 'ready' && Object.keys(event).length === 3) {
      const user = safeEventText(event.user, 64);
      const shell = safeEventText(event.shell, 512);
      if (user !== null && shell !== null && shell.startsWith('/')) {
        return { type: 'ready', user, shell };
      }
    }
    if (event.type === 'exit' && Object.keys(event).length === 3) {
      const exitCode = event.exitCode;
      const signal = event.signal;
      if (
        Number.isSafeInteger(exitCode) &&
        typeof exitCode === 'number' &&
        exitCode >= 0 &&
        exitCode <= 4_294_967_295 &&
        (signal === null || safeEventText(signal, 128) !== null)
      ) {
        return { type: 'exit', exitCode, signal: signal as string | null };
      }
    }
    if (event.type === 'error' && Object.keys(event).length === 2) {
      const message = safeEventText(event.message, 512);
      if (message !== null) return { type: 'error', message };
    }
  } catch {
    return null;
  }
  return null;
}

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : 'Helix could not open the terminal.';
}

function isExpiredSession(error: unknown): boolean {
  return error instanceof ApiError &&
    (error.code === 'authentication_required' || error.code === 'csrf_rejected');
}

function terminalTheme(host: HTMLElement): ITheme {
  const style = globalThis.getComputedStyle(host);
  const background = style.getPropertyValue('--terminal-background').trim() || '#0b0d10';
  const foreground = style.getPropertyValue('--terminal-foreground').trim() || '#e8ebdf';
  const accent = style.getPropertyValue('--accent').trim() || '#d7f64d';
  return {
    background,
    foreground,
    cursor: accent,
    cursorAccent: background,
    selectionBackground: `${accent}40`,
    selectionInactiveBackground: `${accent}28`,
    scrollbarSliderBackground: `${foreground}33`,
    scrollbarSliderHoverBackground: `${foreground}55`,
    scrollbarSliderActiveBackground: `${accent}66`,
    black: '#111318',
    red: '#ff7168',
    green: '#b7df65',
    yellow: '#f0c96b',
    blue: '#7bb6ff',
    magenta: '#d9a3ff',
    cyan: '#69d7db',
    white: '#d9ddd4',
    brightBlack: '#6e746e',
    brightRed: '#ff948d',
    brightGreen: '#d7f68d',
    brightYellow: '#ffe39a',
    brightBlue: '#a7ccff',
    brightMagenta: '#e8c5ff',
    brightCyan: '#9ce8ea',
    brightWhite: '#ffffff',
  };
}

function detachIo(runtime: TerminalRuntime): void {
  runtime.dataDisposable?.dispose();
  runtime.binaryDisposable?.dispose();
  runtime.resizeDisposable?.dispose();
  runtime.dataDisposable = null;
  runtime.binaryDisposable = null;
  runtime.resizeDisposable = null;
  if (runtime.keepalive !== null) globalThis.clearInterval(runtime.keepalive);
  runtime.keepalive = null;
  runtime.pendingOutputBytes = 0;
}

function stopSocket(runtime: TerminalRuntime): void {
  detachIo(runtime);
  const socket = runtime.socket;
  runtime.socket = null;
  if (socket !== null && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify({ type: 'close' }));
    socket.close(1000, 'terminal closed');
  } else if (socket !== null && socket.readyState === WebSocket.CONNECTING) {
    socket.close();
  }
}

export function TerminalPage({ csrfToken, canOpen, onSessionExpired }: TerminalPageProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const runtimeRef = useRef<TerminalRuntime | null>(null);
  const generationRef = useRef(0);
  const findInputRef = useRef<HTMLInputElement | null>(null);
  const findTermRef = useRef('');
  const hintTimerRef = useRef<number | null>(null);
  const chromeRef = useRef<TerminalChrome>({
    copy() {},
    paste() {},
    openFind() {},
    runFind() {},
    closeFind() {},
    findOpen: false,
    adjustFont() {},
  });
  const [status, setStatus] = useState<TerminalStatus | null>(null);
  const [phase, setPhase] = useState<TerminalPhase>('loading');
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [sessionLabel, setSessionLabel] = useState<string | null>(null);
  const [findOpen, setFindOpen] = useState(false);
  const [findTerm, setFindTerm] = useState('');
  const [findHit, setFindHit] = useState<boolean | null>(null);
  const [fontSize, setFontSize] = useState(readStoredTerminalFontSize);
  const [bellFlash, setBellFlash] = useState(false);
  const [hint, setHint] = useState<string | null>(null);
  const [terminalReady, setTerminalReady] = useState(false);

  const showHint = (message: string): void => {
    setHint(message);
    if (hintTimerRef.current !== null) globalThis.clearTimeout(hintTimerRef.current);
    hintTimerRef.current = globalThis.setTimeout(() => {
      setHint(null);
      hintTimerRef.current = null;
    }, 4_000);
  };

  chromeRef.current = {
    findOpen,
    copy: () => {
      const runtime = runtimeRef.current;
      if (runtime === null) return;
      const text = runtime.terminal.getSelection();
      if (text.length === 0) {
        showHint('Select text in the terminal first.');
        return;
      }
      void writeClipboardText(text).then((copied) => {
        showHint(copied ? 'Copied.' : 'Copy failed. Select the text and use your browser copy command.');
      });
    },
    paste: () => {
      const runtime = runtimeRef.current;
      if (runtime === null || runtime.socket?.readyState !== WebSocket.OPEN) return;
      void readClipboardText().then((text) => {
        if (text !== null && text.length > 0) {
          runtime.terminal.paste(text);
          runtime.terminal.focus();
          return;
        }
        showHint('Press Ctrl+V while the terminal is focused to paste.');
        runtime.terminal.focus();
      });
    },
    openFind: () => {
      setFindOpen(true);
      requestAnimationFrame(() => findInputRef.current?.focus());
    },
    runFind: (direction, incremental = false) => {
      const runtime = runtimeRef.current;
      const term = findTermRef.current;
      if (runtime === null || term.length === 0) {
        setFindHit(null);
        return;
      }
      const hit = direction === 'previous'
        ? runtime.search.findPrevious(term, { caseSensitive: false })
        : runtime.search.findNext(term, { caseSensitive: false, incremental });
      setFindHit(hit);
    },
    closeFind: () => {
      runtimeRef.current?.search.clearDecorations();
      setFindOpen(false);
      setFindHit(null);
      runtimeRef.current?.terminal.focus();
    },
    adjustFont: (delta) => {
      const runtime = runtimeRef.current;
      if (runtime === null) return;
      const current = Number(runtime.terminal.options.fontSize) || TERMINAL_FONT_SIZE_DEFAULT;
      const next = delta === 0 ? TERMINAL_FONT_SIZE_DEFAULT : current + delta;
      const size = saveTerminalFontSize(next);
      runtime.terminal.options.fontSize = size;
      try { runtime.fit.fit(); } catch { /* The next visible resize retries. */ }
      setFontSize(size);
    },
  };

  useEffect(() => {
    return () => {
      if (hintTimerRef.current !== null) globalThis.clearTimeout(hintTimerRef.current);
    };
  }, []);

  useEffect(() => {
    if (!canOpen) {
      setError('This account is not allowed to open the host terminal.');
      setPhase('unavailable');
      return;
    }
    const controller = new AbortController();
    void getTerminalStatus(csrfToken, controller.signal)
      .then((next) => {
        setStatus(next);
        setPhase(next.availability === 'available' ? 'locked' : 'unavailable');
      })
      .catch((nextError: unknown) => {
        if (controller.signal.aborted) return;
        if (isExpiredSession(nextError)) onSessionExpired();
        else {
          setError(describeError(nextError));
          setPhase('unavailable');
        }
      });
    return () => controller.abort();
  }, [canOpen, csrfToken, onSessionExpired]);

  useEffect(() => {
    const host = hostRef.current;
    if (host === null || status?.availability !== 'available' || runtimeRef.current !== null) return;
    const initialFontSize = readStoredTerminalFontSize();
    const terminal = new Terminal({
      allowProposedApi: false,
      convertEol: false,
      cursorBlink: true,
      cursorStyle: 'block',
      cursorInactiveStyle: 'outline',
      drawBoldTextInBrightColors: true,
      fontFamily: '"Cascadia Mono", "JetBrains Mono", "SFMono-Regular", Consolas, monospace',
      fontSize: initialFontSize,
      lineHeight: 1.25,
      logLevel: 'error',
      macOptionIsMeta: true,
      macOptionClickForcesSelection: true,
      minimumContrastRatio: 4.5,
      scrollback: 10_000,
      scrollOnEraseInDisplay: true,
      tabStopWidth: 8,
      wordSeparator: ' ()[]{}\'"`<>|;&=',
      theme: terminalTheme(host),
      linkHandler: {
        activate(event, text) {
          event.preventDefault();
          openSafeHttpUrl(text);
        },
        allowNonHttpProtocols: false,
      },
    });
    const fit = new FitAddon();
    const search = new SearchAddon();
    terminal.loadAddon(fit);
    terminal.loadAddon(search);
    terminal.loadAddon(new WebLinksAddon((event, uri) => {
      event.preventDefault();
      openSafeHttpUrl(uri);
    }));
    terminal.open(host);
    terminal.attachCustomKeyEventHandler((event) => {
      if (event.type === 'keydown' && event.key === 'Escape' && chromeRef.current.findOpen) {
        event.preventDefault();
        chromeRef.current.closeFind();
        return false;
      }
      const action = terminalKeyAction(event);
      if (action === 'pass') return true;
      if (action === 'find-next' || action === 'find-previous') {
        if (!chromeRef.current.findOpen) return true;
        event.preventDefault();
        if (event.type === 'keydown') chromeRef.current.runFind(action === 'find-previous' ? 'previous' : 'next');
        return false;
      }
      if (action === 'tab') {
        event.preventDefault();
        return true;
      }
      event.preventDefault();
      if (event.type !== 'keydown') return false;
      if (action === 'copy') chromeRef.current.copy();
      else if (action === 'paste') chromeRef.current.paste();
      else if (action === 'find') chromeRef.current.openFind();
      else if (action === 'font-up') chromeRef.current.adjustFont(1);
      else if (action === 'font-down') chromeRef.current.adjustFont(-1);
      else if (action === 'font-reset') chromeRef.current.adjustFont(0);
      return false;
    });
    terminal.attachCustomWheelEventHandler((event) => {
      if (!event.ctrlKey && !event.metaKey) return true;
      event.preventDefault();
      if (event.deltaY === 0) return false;
      chromeRef.current.adjustFont(event.deltaY < 0 ? 1 : -1);
      return false;
    });
    const applyTheme = (): void => {
      terminal.options.theme = { ...terminalTheme(host) };
    };
    const themeObserver = typeof MutationObserver === 'undefined'
      ? null
      : new MutationObserver(applyTheme);
    themeObserver?.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    globalThis.addEventListener(DASHBOARD_PREFERENCES_EVENT, applyTheme);
    let bellTimer: number | null = null;
    const bellDisposable = terminal.onBell(() => {
      setBellFlash(true);
      if (bellTimer !== null) globalThis.clearTimeout(bellTimer);
      bellTimer = globalThis.setTimeout(() => {
        setBellFlash(false);
        bellTimer = null;
      }, 180);
    });
    const titleDisposable = terminal.onTitleChange((title) => {
      const safe = safeTerminalTitle(title);
      if (safe !== null) setSessionLabel(safe);
    });
    const runtime: TerminalRuntime = {
      terminal,
      fit,
      search,
      socket: null,
      resizeObserver: null,
      dataDisposable: null,
      binaryDisposable: null,
      resizeDisposable: null,
      keepalive: null,
      pendingOutputBytes: 0,
    };
    runtimeRef.current = runtime;
    setFontSize(initialFontSize);
    setTerminalReady(true);
    const resizeObserver = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => {
      try { fit.fit(); } catch { /* The next visible resize retries. */ }
    });
    resizeObserver?.observe(host);
    runtime.resizeObserver = resizeObserver;
    requestAnimationFrame(() => {
      try { fit.fit(); } catch { /* The connect action retries after layout. */ }
    });
    return () => {
      generationRef.current += 1;
      stopSocket(runtime);
      resizeObserver?.disconnect();
      themeObserver?.disconnect();
      globalThis.removeEventListener(DASHBOARD_PREFERENCES_EVENT, applyTheme);
      if (bellTimer !== null) globalThis.clearTimeout(bellTimer);
      bellDisposable.dispose();
      titleDisposable.dispose();
      terminal.dispose();
      runtimeRef.current = null;
      setTerminalReady(false);
    };
  }, [status]);

  useEffect(() => {
    findTermRef.current = findTerm;
  }, [findTerm]);

  const disconnect = (): void => {
    const runtime = runtimeRef.current;
    if (runtime !== null) stopSocket(runtime);
    generationRef.current += 1;
    setSessionLabel(null);
    setPhase('ended');
  };

  const connect = async (): Promise<void> => {
    const runtime = runtimeRef.current;
    if (runtime === null || password.length === 0 || phase === 'authorizing' || phase === 'connecting') return;
    stopSocket(runtime);
    const generation = generationRef.current + 1;
    generationRef.current = generation;
    setError(null);
    setSessionLabel(null);
    setPhase('authorizing');
    try {
      runtime.fit.fit();
      const ticket = await requestTerminalTicket(
        password,
        Math.max(20, Math.min(400, runtime.terminal.cols)),
        Math.max(5, Math.min(200, runtime.terminal.rows)),
        csrfToken,
      );
      if (generationRef.current !== generation) return;
      setPassword('');
      setPhase('connecting');
      runtime.terminal.reset();
      const socket = new WebSocket(
        terminalWebSocketUrl(ticket.connectPath, globalThis.location.href),
        ticket.subprotocol,
      );
      socket.binaryType = 'arraybuffer';
      runtime.socket = socket;
      runtime.dataDisposable = runtime.terminal.onData((data) => {
        if (socket.readyState === WebSocket.OPEN) socket.send(new TextEncoder().encode(data));
      });
      runtime.binaryDisposable = runtime.terminal.onBinary((data) => {
        if (socket.readyState === WebSocket.OPEN) socket.send(encodeBinaryPtyPayload(data));
      });
      runtime.resizeDisposable = runtime.terminal.onResize(({ cols, rows }) => {
        if (socket.readyState === WebSocket.OPEN) {
          socket.send(JSON.stringify({ type: 'resize', columns: cols, rows }));
        }
      });
      runtime.keepalive = globalThis.setInterval(() => {
        if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: 'keepalive' }));
      }, 15_000);
      socket.addEventListener('open', () => {
        if (generationRef.current !== generation) return;
        if (socket.protocol !== ticket.subprotocol) {
          setError('The terminal connection did not negotiate Helix’s protected protocol.');
          socket.close();
          return;
        }
        try { runtime.fit.fit(); } catch { /* Existing PTY dimensions remain valid. */ }
      });
      socket.addEventListener('message', (event) => {
        if (generationRef.current !== generation) return;
        if (event.data instanceof ArrayBuffer) {
          const output = new Uint8Array(event.data);
          const nextBacklog = nextTerminalOutputBacklog(runtime.pendingOutputBytes, output.byteLength);
          if (nextBacklog === null) {
            setError('Terminal output exceeded the browser render budget. The shell was closed to keep Helix responsive.');
            setPhase('ended');
            socket.close(1009, 'terminal output backlog');
            return;
          }
          runtime.pendingOutputBytes = nextBacklog;
          runtime.terminal.write(output, () => {
            runtime.pendingOutputBytes = Math.max(0, runtime.pendingOutputBytes - output.byteLength);
          });
          return;
        }
        if (typeof event.data !== 'string') {
          setError('The terminal returned an unsupported message.');
          socket.close();
          return;
        }
        const hostEvent = parseHostTerminalEvent(event.data);
        if (hostEvent === null) {
          setError('The terminal returned a message Helix could not safely understand.');
          socket.close();
        } else if (hostEvent.type === 'ready') {
          setSessionLabel(`${hostEvent.user} · ${hostEvent.shell}`);
          setPhase('connected');
          runtime.terminal.focus();
        } else if (hostEvent.type === 'exit') {
          const signal = hostEvent.signal === null ? '' : ` (${hostEvent.signal})`;
          setSessionLabel(`Shell exited with code ${hostEvent.exitCode}${signal}`);
          setPhase('ended');
        } else if (hostEvent.type === 'error') {
          setError(hostEvent.message);
          setPhase('ended');
        }
      });
      socket.addEventListener('error', () => {
        if (generationRef.current === generation) {
          setError('The live terminal connection failed. The rest of Helix is still available.');
        }
      });
      socket.addEventListener('close', () => {
        if (generationRef.current !== generation) return;
        detachIo(runtime);
        runtime.socket = null;
        setPhase((current) => current === 'connected' || current === 'connecting' ? 'ended' : current);
      });
    } catch (nextError) {
      if (generationRef.current !== generation) return;
      if (isExpiredSession(nextError)) onSessionExpired();
      else setError(describeError(nextError));
      setPhase(status?.availability === 'available' ? 'locked' : 'unavailable');
    }
  };

  const busy = phase === 'authorizing' || phase === 'connecting';
  const connected = phase === 'connected';
  const locked = phase === 'locked' || phase === 'ended' || busy;
  const runShortcut = (command: string): void => {
    const runtime = runtimeRef.current;
    if (!connected || runtime?.socket?.readyState !== WebSocket.OPEN) return;
    runtime.socket.send(new TextEncoder().encode(`${command}\r`));
    runtime.terminal.focus();
  };
  return (
    <div class="page page--terminal">
      <PageHead
        title="Terminal"
        detail="A direct PTY on the Linux host, opened only after a fresh password check."
        actions={<span class={`state-label state-label--${connected ? 'good' : phase === 'unavailable' ? 'warning' : 'neutral'}`}><span class={`status-dot ${connected ? 'status-dot--good' : ''}`} />{connected ? 'Connected' : busy ? 'Connecting' : phase === 'unavailable' ? 'Unavailable' : 'Locked'}</span>}
      />
      <InlineError message={error} />
      <section class="terminal-safety surface">
        <div><Icon name="terminal" size={20} /><span><strong>Runs as the configured Linux user</strong><p>This is a real shell, not a command simulator. Tab completes paths and commands the same way bash would over SSH. <code>sudo</code> still follows the host’s normal policy and may ask for the Linux account password.</p></span></div>
        <InfoTip text="Helix audits only terminal authorization and connection lifecycle. It does not store commands, keystrokes, output, or environment values. Closing this page ends the PTY, so use tmux or a system service for long-running work." />
      </section>

      <section class="terminal-frame surface" aria-busy={busy}>
        <header class="terminal-toolbar">
          <div><span class="terminal-led" aria-hidden="true" /><strong>{sessionLabel ?? 'Host shell'}</strong>{hint !== null && <span class="terminal-hint" role="status">{hint}</span>}</div>
          <div class="terminal-toolbar-actions">
            <button class="button button--quiet" type="button" disabled={!terminalReady} onClick={() => chromeRef.current.openFind()}><Icon name="search" size={14} />Find</button>
            <button class="button button--quiet" type="button" disabled={!connected} onClick={() => chromeRef.current.copy()}>Copy</button>
            <button class="button button--quiet" type="button" disabled={!connected} onClick={() => chromeRef.current.paste()}>Paste</button>
            <button class="button button--quiet" type="button" disabled={!terminalReady} onClick={() => chromeRef.current.adjustFont(-1)} aria-label="Smaller terminal text">A−</button>
            <button class="button button--quiet" type="button" disabled={!terminalReady} onClick={() => chromeRef.current.adjustFont(1)} aria-label="Larger terminal text">A+</button>
            <button class="button button--quiet" type="button" disabled={!terminalReady} onClick={() => runtimeRef.current?.terminal.clear()}><Icon name="trash" size={14} />Clear view</button>
            <button class="button button--danger" type="button" disabled={!connected && phase !== 'connecting'} onClick={disconnect}><Icon name="close" size={14} />Disconnect</button>
          </div>
        </header>
        {connected && (
          <div class="terminal-shortcuts" aria-label="Read-only command shortcuts">
            <span>Quick checks</span>
            <button type="button" onClick={() => runShortcut('ls -lah --color=auto')}>Files</button>
            <button type="button" onClick={() => runShortcut('df -h --output=source,size,used,avail,pcent,target')}>Drives</button>
            <button type="button" onClick={() => runShortcut('systemctl --failed --no-pager')}>Failed services</button>
            <small>Output stays terminal-native so columns, colors, prompts, and interactive tools render correctly.</small>
          </div>
        )}
        {findOpen && (
          <form
            class="terminal-find"
            onSubmit={(event) => {
              event.preventDefault();
              chromeRef.current.runFind('next');
            }}
          >
            <Icon name="search" size={14} />
            <input
              ref={findInputRef}
              type="search"
              value={findTerm}
              placeholder="Find in scrollback"
              aria-label="Find in terminal scrollback"
              onInput={(event) => {
                const next = event.currentTarget.value;
                setFindTerm(next);
                findTermRef.current = next;
                chromeRef.current.runFind('next', true);
              }}
            />
            <span class={`terminal-find-status${findHit === false ? ' is-miss' : ''}`}>
              {findTerm.length === 0 ? '' : findHit === false ? 'No match' : findHit === true ? 'Match' : ''}
            </span>
            <button type="button" class="button button--quiet" onClick={() => chromeRef.current.runFind('previous')}>Prev</button>
            <button type="submit" class="button button--quiet">Next</button>
            <button type="button" class="button button--quiet" onClick={() => chromeRef.current.closeFind()}>Close</button>
          </form>
        )}
        <div class="terminal-stage">
          <div
            ref={hostRef}
            class={`terminal-host${bellFlash ? ' is-bell' : ''}`}
            aria-label="Linux host terminal"
            onMouseDown={() => {
              if (connected) runtimeRef.current?.terminal.focus();
            }}
          />
          {phase === 'loading' && <div class="terminal-lock"><Icon name="terminal" size={28} /><strong>Checking the host terminal…</strong></div>}
          {phase === 'unavailable' && <div class="terminal-lock"><Icon name="warning" size={28} /><strong>Terminal service unavailable</strong><p>{status?.detail ?? 'Helix could not verify the optional host terminal service.'}</p></div>}
          {locked && !connected && (
            <form class="terminal-lock" onSubmit={(event) => { event.preventDefault(); void connect(); }}>
              <Icon name="terminal" size={28} />
              <strong>{phase === 'ended' ? 'Open a new terminal' : 'Unlock terminal'}</strong>
              <p>Enter the current Helix dashboard password. Authorization expires after 30 seconds and works once.</p>
              <label><span>Current password</span><input type="password" value={password} maxLength={1_024} autoComplete="current-password" disabled={busy} onInput={(event) => setPassword(event.currentTarget.value)} /></label>
              <button class="button button--primary" type="submit" disabled={password.length === 0 || busy}>{phase === 'authorizing' ? 'Checking password…' : phase === 'connecting' ? 'Opening PTY…' : 'Open terminal'}</button>
            </form>
          )}
        </div>
        <footer>
          <span>10,000-line local scrollback · {fontSize}px</span>
          <span>Tab completes in the shell</span>
          <span>Ctrl+Shift+C/V copy/paste · Ctrl+F find · Ctrl+/− font</span>
          <span>Session ends when this page disconnects</span>
          <span>Input and output are not logged by Helix</span>
        </footer>
      </section>
    </div>
  );
}
