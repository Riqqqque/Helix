import { useEffect, useRef } from 'preact/hooks';
import { ApiError } from './api';
import { strandFileUrl, strandHostCall } from './strands-api';
import './strands.css';

interface StrandFrameProps {
  strandId: string;
  uiEntry: string;
  csrfToken: string;
  surface?: 'page' | 'widget';
  title: string;
  onSessionExpired: () => void;
}

interface StrandHostMessage {
  type?: unknown;
  id?: unknown;
  method?: unknown;
  params?: unknown;
}

export function StrandFrame({
  strandId,
  uiEntry,
  csrfToken,
  surface = 'page',
  title,
  onSessionExpired,
}: StrandFrameProps) {
  const frameRef = useRef<HTMLIFrameElement | null>(null);

  useEffect(() => {
    const onMessage = (event: MessageEvent<StrandHostMessage>): void => {
      const frame = frameRef.current;
      if (frame === null || event.source !== frame.contentWindow) return;
      if (event.origin !== 'null' && event.origin !== window.location.origin) return;
      const message = event.data;
      if (!message || message.type !== 'helix-strand' || typeof message.id !== 'string' || typeof message.method !== 'string') {
        return;
      }
      const params = typeof message.params === 'object' && message.params !== null && !Array.isArray(message.params)
        ? message.params as Record<string, unknown>
        : {};
      void strandHostCall(csrfToken, strandId, message.method, params)
        .then((result) => {
          frame.contentWindow?.postMessage({ type: 'helix-strand-result', id: message.id, ok: true, result }, '*');
        })
        .catch((error: unknown) => {
          if (error instanceof ApiError && (error.status === 401 || error.code === 'csrf_rejected')) {
            onSessionExpired();
            return;
          }
          const text = error instanceof Error ? error.message : 'Strand host call failed';
          frame.contentWindow?.postMessage({ type: 'helix-strand-result', id: message.id, ok: false, error: text }, '*');
        });
    };
    window.addEventListener('message', onMessage);
    return () => window.removeEventListener('message', onMessage);
  }, [csrfToken, onSessionExpired, strandId]);

  const src = `${strandFileUrl(strandId, uiEntry)}${surface === 'widget' ? '#helix-widget' : ''}`;
  return (
    <iframe
      ref={frameRef}
      class={surface === 'widget' ? 'strand-frame strand-frame--widget' : 'strand-frame'}
      title={title}
      src={src}
      sandbox="allow-scripts"
      referrerpolicy="no-referrer"
    />
  );
}
