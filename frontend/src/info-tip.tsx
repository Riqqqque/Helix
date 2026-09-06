import { createPortal } from 'preact/compat';
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'preact/hooks';

const bubbleGap = 10;
const viewportMargin = 12;
const maximumBubbleWidth = 300;

export interface InfoTipPlacement {
  left: number;
  top: number;
  width: number;
  arrowLeft: number;
  side: 'above' | 'below';
}

interface TriggerBounds {
  left: number;
  right: number;
  top: number;
  bottom: number;
  width: number;
}

export function placeInfoTip(
  trigger: TriggerBounds,
  bubbleHeight: number,
  viewportWidth: number,
  viewportHeight: number,
): InfoTipPlacement {
  const width = Math.max(0, Math.min(maximumBubbleWidth, viewportWidth - (viewportMargin * 2)));
  const triggerCenter = trigger.left + (trigger.width / 2);
  const left = Math.max(viewportMargin, Math.min(triggerCenter - (width / 2), viewportWidth - viewportMargin - width));
  const fitsBelow = trigger.bottom + bubbleGap + bubbleHeight <= viewportHeight - viewportMargin;
  const fitsAbove = trigger.top - bubbleGap - bubbleHeight >= viewportMargin;
  const side = fitsBelow || !fitsAbove ? 'below' : 'above';
  const unclampedTop = side === 'below' ? trigger.bottom + bubbleGap : trigger.top - bubbleGap - bubbleHeight;
  const top = Math.max(viewportMargin, Math.min(unclampedTop, viewportHeight - viewportMargin - bubbleHeight));
  const arrowLeft = Math.max(10, Math.min(triggerCenter - left, width - 10));

  return { left, top, width, arrowLeft, side };
}

export function InfoTip({ text, label = 'More information' }: { text: string; label?: string }) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const bubbleRef = useRef<HTMLSpanElement>(null);
  const [open, setOpen] = useState(false);
  const [placement, setPlacement] = useState<InfoTipPlacement | null>(null);

  const updatePlacement = useCallback((): void => {
    const trigger = triggerRef.current;
    const bubble = bubbleRef.current;
    if (trigger === null || bubble === null) return;

    const bounds = trigger.getBoundingClientRect();
    const next = placeInfoTip(bounds, bubble.getBoundingClientRect().height, window.innerWidth, window.innerHeight);
    setPlacement(next);
  }, []);

  useLayoutEffect(() => {
    if (!open) {
      setPlacement(null);
      return;
    }
    updatePlacement();
  }, [open, text, updatePlacement]);

  useEffect(() => {
    if (!open) return;
    const reposition = (): void => updatePlacement();
    window.addEventListener('resize', reposition);
    window.addEventListener('scroll', reposition, true);
    return () => {
      window.removeEventListener('resize', reposition);
      window.removeEventListener('scroll', reposition, true);
    };
  }, [open, updatePlacement]);

  const closeAfterPointer = (): void => {
    if (document.activeElement !== triggerRef.current) setOpen(false);
  };

  return (
    <>
      <button
        ref={triggerRef}
        class="info-tip"
        type="button"
        aria-label={`${label}: ${text}`}
        aria-expanded={open}
        onPointerEnter={() => setOpen(true)}
        onPointerLeave={closeAfterPointer}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            setOpen(false);
            event.currentTarget.blur();
          }
        }}
      >
        i
      </button>
      {open && typeof document !== 'undefined' && createPortal(
        <span
          ref={bubbleRef}
          class="info-tip__bubble"
          role="tooltip"
          data-side={placement?.side ?? 'below'}
          style={{
            left: `${placement?.left ?? viewportMargin}px`,
            top: `${placement?.top ?? viewportMargin}px`,
            width: `${placement?.width ?? Math.max(0, Math.min(maximumBubbleWidth, window.innerWidth - (viewportMargin * 2)))}px`,
            visibility: placement === null ? 'hidden' : 'visible',
            '--info-tip-arrow-left': `${placement?.arrowLeft ?? 16}px`,
          }}
        >
          {text}
        </span>,
        document.body,
      )}
    </>
  );
}
