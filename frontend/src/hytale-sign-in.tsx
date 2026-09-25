import { useState } from 'preact/hooks';
import { safeHytaleUrl, type HytaleAuthPrompt } from './control-api';
import { Icon } from './icons';
import './hytale-sign-in.css';

const STAGE_LINK = /https:\/\/[A-Za-z0-9.-]+\/[A-Za-z0-9./?=&_%:~+-]*/u;

/**
 * Job progress text. A Hytale sign-in link inside it becomes a real link, but
 * only after the same https hytale.com check; any other text stays plain.
 */
export function StageText({ text }: { text: string }) {
  const match = STAGE_LINK.exec(text);
  const url = match === null ? null : safeHytaleUrl(match[0]);
  if (match === null || url === null) return <>{text}</>;
  return (
    <>
      {text.slice(0, match.index)}
      <a href={url} target="_blank" rel="noreferrer noopener">{match[0]}</a>
      {text.slice(match.index + match[0].length)}
    </>
  );
}

/**
 * Shows the Hytale account sign-in a server is waiting for. The link is only
 * rendered when it is an https hytale.com URL (checked again here, after the
 * broker's own validation).
 */
export function HytaleSignIn({ auth, running }: { auth: HytaleAuthPrompt | null; running: boolean }) {
  const [copied, setCopied] = useState(false);
  if (!running || auth === null || auth.state !== 'needs_sign_in') return null;
  const url = safeHytaleUrl(auth.url);
  if (url === null) return null;
  const purpose = auth.stage === 'download'
    ? 'Hytale needs your account to download the server files.'
    : 'Hytale needs your account before players can join this server.';
  return (
    <section class="hytale-sign-in" aria-label="Hytale sign-in">
      <Icon name="user" size={18} />
      <div>
        <p role="status"><strong>Sign in to Hytale.</strong> {purpose} Open the link, sign in, and confirm the code. This page updates by itself once Hytale accepts it.</p>
        <div class="hytale-sign-in__actions">
          <a class="button button--primary" href={url} target="_blank" rel="noreferrer noopener">Open Hytale sign-in <Icon name="external" size={14} /></a>
          {auth.code !== null && (
            <span class="hytale-sign-in__code">
              Code <code>{auth.code}</code>
              <button class="button button--quiet" type="button" onClick={() => { void navigator.clipboard?.writeText(auth.code ?? '').then(() => setCopied(true), () => setCopied(false)); }}>{copied ? 'Copied' : 'Copy'}</button>
            </span>
          )}
        </div>
        <small>Helix keeps the sign-in so you only do this once; the server stores it encrypted in its data folder.</small>
      </div>
    </section>
  );
}
