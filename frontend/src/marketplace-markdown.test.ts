import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { renderMarketplaceBody } from './marketplace-markdown';

describe('marketplace markdown', () => {
  it('renders headings, lists, emphasis, and https links', () => {
    const markup = render(renderMarketplaceBody(
      '# Title\n\n**Bold** and *italic* plus [docs](https://modrinth.com/mod/example).\n\n- one\n- two\n\n```\ncode\n```\n',
      'markdown',
    ));
    expect(markup).toContain('<h3>Title</h3>');
    expect(markup).toContain('<strong>Bold</strong>');
    expect(markup).toContain('<em>italic</em>');
    expect(markup).toContain('href="https://modrinth.com/mod/example"');
    expect(markup).toContain('<li>');
    expect(markup).toContain('<pre><code>code</code></pre>');
  });

  it('does not turn javascript or raw html into executable markup', () => {
    const markup = render(renderMarketplaceBody(
      '[x](javascript:alert(1))\n\n<script>alert(1)</script>\n\n<img src="https://evil.test/x.png">\n\n![ok](https://cdn.modrinth.com/data/a/icon.png)',
      'markdown',
    ));
    expect(markup).not.toContain('javascript:');
    expect(markup).not.toContain('<script');
    expect(markup).toContain('https://cdn.modrinth.com/data/a/icon.png');
    expect(markup).not.toContain('https://evil.test/x.png');
  });

  it('strips scripts from CurseForge html descriptions', () => {
    const markup = render(renderMarketplaceBody(
      '<p>Hello</p><script>alert(1)</script><a href="javascript:alert(1)">no</a><p>World</p>',
      'html',
    ));
    expect(markup).toContain('Hello');
    expect(markup).toContain('World');
    expect(markup).not.toContain('<script');
    expect(markup).not.toContain('javascript:');
  });

  it('turns CurseForge html headings, lists, and safe images into readable markup', () => {
    const markup = render(renderMarketplaceBody(
      '<h2>Features</h2><ul><li>One</li><li>Two</li></ul><p><strong>Bold</strong> and <a href="https://www.curseforge.com/minecraft/mc-mods/example">docs</a></p><img src="https://media.forgecdn.net/avatars/1/icon.png" alt="icon">',
      'html',
    ));
    expect(markup).toContain('<h4>Features</h4>');
    expect(markup).toContain('<li>');
    expect(markup).toContain('<strong>Bold</strong>');
    expect(markup).toContain('href="https://www.curseforge.com/minecraft/mc-mods/example"');
    expect(markup).toContain('src="https://media.forgecdn.net/avatars/1/icon.png"');
  });
});
