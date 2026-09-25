import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { parseHytaleAuth, safeHytaleUrl } from './control-api';
import { HytaleSignIn, StageText } from './hytale-sign-in';

const prompt = parseHytaleAuth({
  state: 'needs_sign_in',
  stage: 'download',
  url: 'https://oauth.accounts.hytale.com/oauth2/device/verify?user_code=ABCD-1234',
  code: 'ABCD-1234',
});

describe('Hytale sign-in', () => {
  it('shows the validated link and code while a running server waits for sign-in', () => {
    const html = render(<HytaleSignIn auth={prompt} running={true} />);
    expect(html).toContain('Sign in to Hytale.');
    expect(html).toContain('download the server files');
    expect(html).toContain('href="https://oauth.accounts.hytale.com/oauth2/device/verify?user_code=ABCD-1234"');
    expect(html).toContain('rel="noreferrer noopener"');
    expect(html).toContain('ABCD-1234');
  });

  it('stays hidden once signed in, when stopped, or without evidence', () => {
    expect(render(<HytaleSignIn auth={prompt} running={false} />)).toBe('');
    expect(render(<HytaleSignIn auth={parseHytaleAuth({ state: 'signed_in' })} running={true} />)).toBe('');
    expect(render(<HytaleSignIn auth={null} running={true} />)).toBe('');
  });

  it('never links anywhere but https hytale.com', () => {
    for (const url of [
      'http://oauth.accounts.hytale.com/verify',
      'https://hytale.com.evil.example/verify',
      'https://evilhytale.com/verify',
      'https://user@oauth.accounts.hytale.com/verify',
      'https://oauth.accounts.hytale.com:8443/verify',
      'javascript:alert(1)',
    ]) {
      expect(safeHytaleUrl(url)).toBeNull();
      const forged = parseHytaleAuth({ state: 'needs_sign_in', url, code: 'WXYZ' });
      expect(forged?.state).toBe('unknown');
      expect(render(<HytaleSignIn auth={forged} running={true} />)).toBe('');
    }
    expect(parseHytaleAuth({ state: 'needs_sign_in', url: 'https://hytale.com/device', code: '<b>' })?.code).toBeNull();
  });

  it('links only Hytale sign-in URLs inside job progress text', () => {
    const linked = render(<StageText text="Sign in to Hytale to download the server files: open https://oauth.accounts.hytale.com/oauth2/device/verify?user_code=WXYZ and confirm code WXYZ" />);
    expect(linked).toContain('<a href="https://oauth.accounts.hytale.com/oauth2/device/verify?user_code=WXYZ"');
    expect(linked).toContain('and confirm code WXYZ');
    const plain = render(<StageText text="Downloading from https://example.com/file and https://hytale.com.evil.example/x" />);
    expect(plain).not.toContain('<a');
    expect(render(<StageText text="Installing Hytale and waiting for first boot" />)).toBe('Installing Hytale and waiting for first boot');
  });
});
