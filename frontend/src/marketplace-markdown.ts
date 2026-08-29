import type { ComponentChildren, VNode } from 'preact';
import { h } from 'preact';

const MAX_NODES = 2_000;
const MAX_URL = 1_024;

export type MarketplaceBodyFormat = 'plain_text' | 'markdown' | 'html';

function isSafeHttps(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === 'https:' && parsed.username === '' && parsed.password === '' && url.length <= MAX_URL;
  } catch {
    return false;
  }
}

function isAllowedImageHost(url: string): boolean {
  if (!isSafeHttps(url)) return false;
  const host = new URL(url).hostname;
  return host === 'cdn.modrinth.com'
    || host === 'cdn-raw.modrinth.com'
    || host === 'media.forgecdn.net'
    || host.endsWith('.forgecdn.net')
    || host === 'i.imgur.com'
    || host === 'imgur.com'
    || host === 'raw.githubusercontent.com'
    || host === 'user-images.githubusercontent.com';
}

function consumeInline(text: string, keyPrefix: string): ComponentChildren[] {
  const nodes: ComponentChildren[] = [];
  let remaining = text;
  let token = 0;
  while (remaining.length > 0 && token < 400) {
    token += 1;
    const patterns: Array<{ regex: RegExp; render: (match: RegExpExecArray) => ComponentChildren }> = [
      {
        regex: /^`([^`\n]{1,512})`/,
        render: (match) => h('code', { key: `${keyPrefix}-c${token}` }, match[1]),
      },
      {
        regex: /^!\[([^\]]{0,256})\]\((https:[^)\s]{1,1024})\)/,
        render: (match) => {
          const href = match[2] ?? '';
          const label = match[1] ?? '';
          if (isAllowedImageHost(href)) {
            return h('img', { key: `${keyPrefix}-i${token}`, src: href, alt: label, loading: 'lazy', referrerPolicy: 'no-referrer' });
          }
          if (isSafeHttps(href)) {
            return h('a', { key: `${keyPrefix}-il${token}`, href, target: '_blank', rel: 'noreferrer' }, label || 'Image');
          }
          return label;
        },
      },
      {
        regex: /^\[([^\]]{0,256})\]\(([^)\s]{1,1024})\)/,
        render: (match) => {
          const href = match[2] ?? '';
          const label = match[1] ?? href;
          return isSafeHttps(href)
            ? h('a', { key: `${keyPrefix}-a${token}`, href, target: '_blank', rel: 'noreferrer' }, label)
            : label;
        },
      },
      {
        regex: /^\*\*([^*]{1,512})\*\*/,
        render: (match) => h('strong', { key: `${keyPrefix}-s${token}` }, match[1]),
      },
      {
        regex: /^__([^_]{1,512})__/,
        render: (match) => h('strong', { key: `${keyPrefix}-s${token}` }, match[1]),
      },
      {
        regex: /^\*([^*]{1,512})\*/,
        render: (match) => h('em', { key: `${keyPrefix}-e${token}` }, match[1]),
      },
    ];
    let matched = false;
    for (const pattern of patterns) {
      const match = pattern.regex.exec(remaining);
      if (match === null) continue;
      nodes.push(pattern.render(match));
      remaining = remaining.slice(match[0].length);
      matched = true;
      break;
    }
    if (matched) continue;
    const nextSpecial = remaining.search(/[`!*[_]/u);
    if (nextSpecial <= 0) {
      nodes.push(remaining);
      break;
    }
    nodes.push(remaining.slice(0, nextSpecial));
    remaining = remaining.slice(nextSpecial);
    if (nextSpecial === 0) {
      nodes.push(remaining[0] ?? '');
      remaining = remaining.slice(1);
    }
  }
  return nodes;
}

function decodeEntities(text: string): string {
  return text
    .replace(/&nbsp;/giu, ' ')
    .replace(/&amp;/giu, '&')
    .replace(/&lt;/giu, '<')
    .replace(/&gt;/giu, '>')
    .replace(/&quot;/giu, '"')
    .replace(/&#39;/giu, "'")
    .replace(/&apos;/giu, "'");
}

function stripTags(html: string): string {
  return decodeEntities(
    html
      .replace(/<script[\s\S]*?<\/script>/giu, '')
      .replace(/<style[\s\S]*?<\/style>/giu, '')
      .replace(/<[^>]+>/gu, ' ')
      .replace(/\s+/gu, ' ')
      .trim(),
  );
}

function htmlToMarkdown(html: string): string {
  let text = html
    .replace(/<script[\s\S]*?<\/script>/giu, '')
    .replace(/<style[\s\S]*?<\/style>/giu, '')
    .replace(/<!--[\s\S]*?-->/gu, '');
  text = text.replace(/<a\b([^>]*)>([\s\S]*?)<\/a>/giu, (_full, attrs: string, inner: string) => {
    const href = /href\s*=\s*["']([^"']+)["']/iu.exec(attrs)?.[1] ?? '';
    const label = stripTags(inner) || href;
    return isSafeHttps(href) ? `[${label}](${href})` : label;
  });
  text = text.replace(/<img\b([^>]*)\/?>/giu, (_full, attrs: string) => {
    const src = /src\s*=\s*["']([^"']+)["']/iu.exec(attrs)?.[1] ?? '';
    const alt = /alt\s*=\s*["']([^"']*)["']/iu.exec(attrs)?.[1] ?? '';
    return isSafeHttps(src) ? `![${alt}](${src})` : alt;
  });
  text = text.replace(/<h([1-3])\b[^>]*>([\s\S]*?)<\/h\1>/giu, (_full, level: string, inner: string) => `\n\n${'#'.repeat(Number(level))} ${stripTags(inner)}\n\n`);
  text = text.replace(/<li\b[^>]*>([\s\S]*?)<\/li>/giu, (_full, inner: string) => `- ${stripTags(inner)}\n`);
  text = text.replace(/<\/?(?:ul|ol)\b[^>]*>/giu, '\n');
  text = text.replace(/<blockquote\b[^>]*>([\s\S]*?)<\/blockquote>/giu, (_full, inner: string) => `\n> ${stripTags(inner)}\n`);
  text = text.replace(/<(?:strong|b)\b[^>]*>([\s\S]*?)<\/(?:strong|b)>/giu, (_full, inner: string) => `**${stripTags(inner)}**`);
  text = text.replace(/<(?:em|i)\b[^>]*>([\s\S]*?)<\/(?:em|i)>/giu, (_full, inner: string) => `*${stripTags(inner)}*`);
  text = text.replace(/<code\b[^>]*>([\s\S]*?)<\/code>/giu, (_full, inner: string) => `\`${stripTags(inner)}\``);
  text = text.replace(/<pre\b[^>]*>([\s\S]*?)<\/pre>/giu, (_full, inner: string) => `\n\n\`\`\`\n${stripTags(inner)}\n\`\`\`\n\n`);
  text = text.replace(/<br\s*\/?>/giu, '\n');
  text = text.replace(/<\/p>/giu, '\n\n');
  text = text.replace(/<p\b[^>]*>/giu, '');
  text = text.replace(/<hr\s*\/?>/giu, '\n\n---\n\n');
  text = text.replace(/<[^>]+>/gu, ' ');
  return decodeEntities(text).replace(/[ \t]+\n/gu, '\n').replace(/\n{3,}/gu, '\n\n').trim();
}

export function renderMarketplaceBody(body: string, format: MarketplaceBodyFormat): VNode {
  if (format === 'plain_text') {
    return h(
      'div',
      { class: 'marketplace-markdown marketplace-markdown--plain' },
      body.split(/\n{2,}/u).map((paragraph, index) => h('p', { key: `plain-${index}` }, paragraph)),
    ) as VNode;
  }
  const markdown = format === 'html' ? htmlToMarkdown(body) : body;
  const lines = markdown
    .replace(/<script[\s\S]*?<\/script>/giu, '')
    .replace(/<img\b[^>]*>/giu, '')
    .replace(/\r\n/gu, '\n')
    .split('\n');
  const elements: ComponentChildren[] = [];
  let index = 0;
  let nodes = 0;
  const push = (node: ComponentChildren): void => {
    if (nodes >= MAX_NODES) return;
    elements.push(node);
    nodes += 1;
  };
  while (index < lines.length && nodes < MAX_NODES) {
    const line = lines[index] ?? '';
    if (line.trim().length === 0) {
      index += 1;
      continue;
    }
    if (/^```[\w+-]*\s*$/u.test(line.trim())) {
      const code: string[] = [];
      index += 1;
      while (index < lines.length && !/^```\s*$/u.test((lines[index] ?? '').trim())) {
        code.push(lines[index] ?? '');
        index += 1;
      }
      if (index < lines.length) index += 1;
      push(h('pre', { key: `fence-${index}` }, h('code', null, code.join('\n'))));
      continue;
    }
    const heading = /^(#{1,3})[ \t]+(.+?)\s*$/u.exec(line);
    if (heading) {
      const tag = heading[1]!.length === 1 ? 'h3' : heading[1]!.length === 2 ? 'h4' : 'h5';
      push(h(tag, { key: `h-${index}` }, consumeInline(heading[2] ?? '', `h${index}`)));
      index += 1;
      continue;
    }
    if (/^(?:---+|\*\*\*+)\s*$/u.test(line)) {
      push(h('hr', { key: `hr-${index}` }));
      index += 1;
      continue;
    }
    if (/^>[ \t]?/u.test(line)) {
      const quoted: string[] = [];
      while (index < lines.length && /^>[ \t]?/u.test(lines[index] ?? '')) {
        quoted.push((lines[index] ?? '').replace(/^>[ \t]?/u, ''));
        index += 1;
      }
      push(h('blockquote', { key: `q-${index}` }, quoted.join(' ')));
      continue;
    }
    const bullet = /^[-*+][ \t]+(.+)$/u;
    const ordered = /^(\d{1,4})\.[ \t]+(.+)$/u;
    if (bullet.test(line) || ordered.test(line)) {
      const orderedList = ordered.test(line);
      const items: ComponentChildren[] = [];
      while (index < lines.length) {
        const item = orderedList ? ordered.exec(lines[index] ?? '') : bullet.exec(lines[index] ?? '');
        if (item === null) break;
        items.push(h('li', { key: `li-${index}` }, consumeInline((orderedList ? item[2] : item[1]) ?? '', `li${index}`)));
        index += 1;
      }
      push(h(orderedList ? 'ol' : 'ul', { key: `list-${index}` }, items));
      continue;
    }
    const paragraph = [line];
    index += 1;
    while (index < lines.length) {
      const next = lines[index] ?? '';
      if (
        next.trim().length === 0
        || /^(#{1,3})[ \t]+/u.test(next)
        || /^```/u.test(next.trim())
        || bullet.test(next)
        || ordered.test(next)
        || /^>/u.test(next)
      ) {
        break;
      }
      paragraph.push(next);
      index += 1;
    }
    push(h('p', { key: `p-${index}` }, consumeInline(paragraph.join(' '), `p${index}`)));
  }
  return h('div', { class: 'marketplace-markdown' }, elements) as VNode;
}
