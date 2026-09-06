import { useEffect, useRef, useState } from 'preact/hooks';
import { writeClipboardText } from './terminal-keys';

export type CopyFlash = 'idle' | 'copied' | 'failed';

export const COPY_FLASH_MS = 1_600;

export function copyFlashLabel(flash: CopyFlash, idleLabel = 'Copy'): string {
  if (flash === 'copied') return 'Copied';
  if (flash === 'failed') return 'Couldn’t copy';
  return idleLabel;
}

export function useCopyFlash(resetMs = COPY_FLASH_MS): {
  flash: CopyFlash;
  show: (next: CopyFlash) => void;
} {
  const [flash, setFlash] = useState<CopyFlash>('idle');
  const timer = useRef<number | null>(null);

  useEffect(() => () => {
    if (timer.current !== null) globalThis.clearTimeout(timer.current);
  }, []);

  const show = (next: CopyFlash): void => {
    if (timer.current !== null) globalThis.clearTimeout(timer.current);
    setFlash(next);
    if (next === 'idle') {
      timer.current = null;
      return;
    }
    timer.current = globalThis.setTimeout(() => {
      setFlash('idle');
      timer.current = null;
    }, resetMs);
  };

  return { flash, show };
}

export function CopyButton({
  text,
  idleLabel = 'Copy',
  class: className,
}: {
  text: string;
  idleLabel?: string;
  class?: string;
}) {
  const { flash, show } = useCopyFlash();
  const copied = flash === 'copied';
  const failed = flash === 'failed';
  return (
    <button
      type="button"
      class={`copy-button${copied ? ' is-copied' : ''}${failed ? ' is-failed' : ''}${className !== undefined && className.length > 0 ? ` ${className}` : ''}`}
      aria-live="polite"
      onClick={() => {
        void writeClipboardText(text).then((ok) => show(ok ? 'copied' : 'failed'));
      }}
    >
      {copyFlashLabel(flash, idleLabel)}
    </button>
  );
}
