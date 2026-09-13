import type { ComponentChildren, VNode } from 'preact';
import { h } from 'preact';

const MAX_NODES = 2_000;
const MAX_MARKUP_CHARS = 500_000;
const MAX_HTML_TAG_CHARS = 4_096;
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

const HTML_ENTITY = new Map([
  ['nbsp', ' '],
  ['amp', '&'],
  ['lt', '<'],
  ['gt', '>'],
  ['quot', '"'],
  ['apos', "'"],
  ['#39', "'"],
]);

function decodeEntitiesOnce(text: string): string {
  let decoded = '';
  let index = 0;
  while (index < text.length) {
    if (text[index] !== '&') {
      decoded += text[index];
      index += 1;
      continue;
    }
    let end = -1;
    const entityEnd = Math.min(text.length, index + 13);
    for (let cursor = index + 1; cursor < entityEnd; cursor += 1) {
      if (text[cursor] === ';') {
        end = cursor;
        break;
      }
    }
    if (end < 0) {
      decoded += '&';
      index += 1;
      continue;
    }
    const entity = text.slice(index + 1, end);
    const named = HTML_ENTITY.get(entity.toLowerCase());
    let value = named;
    if (value === undefined && /^#(?:x[0-9a-f]+|[0-9]+)$/iu.test(entity)) {
      const hexadecimal = entity[1]?.toLowerCase() === 'x';
      const codePoint = Number.parseInt(entity.slice(hexadecimal ? 2 : 1), hexadecimal ? 16 : 10);
      if (codePoint > 0 && codePoint <= 0x10ffff && !(codePoint >= 0xd800 && codePoint <= 0xdfff)) {
        value = String.fromCodePoint(codePoint);
      }
    }
    if (value === undefined) {
      decoded += text.slice(index, end + 1);
    } else {
      decoded += value;
    }
    index = end + 1;
  }
  return decoded;
}

interface ParsedHtmlTag {
  attributes: Record<string, string>;
  closing: boolean;
  end: number;
  name: string;
  selfClosing: boolean;
}

function parseHtmlTag(source: string, start: number): ParsedHtmlTag | null {
  let end = start + 1;
  const tagEnd = Math.min(source.length, start + MAX_HTML_TAG_CHARS + 1);
  let quote = '';
  while (end < tagEnd) {
    const character = source[end] ?? '';
    if (quote !== '') {
      if (character === quote) quote = '';
    } else if (character === '"' || character === "'") {
      quote = character;
    } else if (character === '>') {
      break;
    }
    end += 1;
  }
  if (end >= tagEnd || source[end] !== '>') return null;
  const content = source.slice(start + 1, end).trim();
  if (content === '' || content.startsWith('!') || content.startsWith('?')) {
    return { attributes: {}, closing: false, end: end + 1, name: '', selfClosing: true };
  }
  let cursor = content.startsWith('/') ? 1 : 0;
  while (cursor < content.length && /\s/u.test(content[cursor] ?? '')) cursor += 1;
  const nameStart = cursor;
  while (cursor < content.length && /[a-z0-9:-]/iu.test(content[cursor] ?? '')) cursor += 1;
  const name = content.slice(nameStart, cursor).toLowerCase();
  if (name === '') return null;
  const attributes: Record<string, string> = {};
  while (cursor < content.length) {
    while (cursor < content.length && /[\s/]/u.test(content[cursor] ?? '')) cursor += 1;
    const attributeStart = cursor;
    while (cursor < content.length && /[a-z0-9_:-]/iu.test(content[cursor] ?? '')) cursor += 1;
    const attribute = content.slice(attributeStart, cursor).toLowerCase();
    if (attribute === '') {
      cursor += 1;
      continue;
    }
    while (cursor < content.length && /\s/u.test(content[cursor] ?? '')) cursor += 1;
    let value = '';
    if (content[cursor] === '=') {
      cursor += 1;
      while (cursor < content.length && /\s/u.test(content[cursor] ?? '')) cursor += 1;
      const delimiter = content[cursor];
      if (delimiter === '"' || delimiter === "'") {
        cursor += 1;
        const valueStart = cursor;
        while (cursor < content.length && content[cursor] !== delimiter) cursor += 1;
        value = content.slice(valueStart, cursor);
        if (cursor < content.length) cursor += 1;
      } else {
        const valueStart = cursor;
        while (cursor < content.length && !/[\s>]/u.test(content[cursor] ?? '')) cursor += 1;
        value = content.slice(valueStart, cursor);
      }
    }
    attributes[attribute] = decodeEntitiesOnce(value);
  }
  return {
    attributes,
    closing: content.startsWith('/'),
    end: end + 1,
    name,
    selfClosing: content.endsWith('/'),
  };
}

interface HtmlFrame {
  children: ComponentChildren[];
  outputTag: string;
  properties: Record<string, unknown>;
  sourceTag: string;
}

const HTML_TAGS: Record<string, string> = {
  a: 'a', b: 'strong', blockquote: 'blockquote', code: 'code', em: 'em', h1: 'h3', h2: 'h4', h3: 'h5',
  i: 'em', li: 'li', ol: 'ol', p: 'p', pre: 'pre', strong: 'strong', ul: 'ul',
};

function renderMarketplaceHtml(body: string): VNode {
  const source = body.slice(0, MAX_MARKUP_CHARS);
  const stack: HtmlFrame[] = [{ children: [], outputTag: 'div', properties: { class: 'marketplace-markdown' }, sourceTag: 'root' }];
  let index = 0;
  let operations = 0;
  let suppressed: 'script' | 'style' | null = null;
  const append = (node: ComponentChildren): void => {
    stack[stack.length - 1]?.children.push(node);
  };
  const close = (name: string): void => {
    const frameIndex = stack.findLastIndex((frame) => frame.sourceTag === name);
    if (frameIndex < 1) return;
    while (stack.length - 1 >= frameIndex) {
      const frame = stack.pop();
      if (frame !== undefined) append(h(frame.outputTag, frame.properties, frame.children));
    }
  };
  while (index < source.length && operations < MAX_NODES) {
    operations += 1;
    if (source.startsWith('<!--', index)) {
      const commentEnd = source.indexOf('-->', index + 4);
      index = commentEnd < 0 ? source.length : commentEnd + 3;
      continue;
    }
    if (source[index] !== '<') {
      const next = source.indexOf('<', index);
      if (suppressed === null) append(decodeEntitiesOnce(source.slice(index, next < 0 ? source.length : next)));
      index = next < 0 ? source.length : next;
      continue;
    }
    const tag = parseHtmlTag(source, index);
    if (tag === null) {
      if (suppressed === null) append('<');
      index += 1;
      continue;
    }
    index = tag.end;
    if (suppressed !== null) {
      if (tag.closing && tag.name === suppressed) suppressed = null;
      continue;
    }
    if (!tag.closing && (tag.name === 'script' || tag.name === 'style')) {
      suppressed = tag.name;
      continue;
    }
    if (tag.closing) {
      close(tag.name);
      continue;
    }
    if (tag.name === 'br') {
      append(h('br', null));
      continue;
    }
    if (tag.name === 'hr') {
      append(h('hr', null));
      continue;
    }
    if (tag.name === 'img') {
      const src = tag.attributes.src ?? '';
      const alt = tag.attributes.alt ?? '';
      append(isAllowedImageHost(src)
        ? h('img', { src, alt, loading: 'lazy', referrerPolicy: 'no-referrer' })
        : alt);
      continue;
    }
    const outputTag = HTML_TAGS[tag.name];
    if (outputTag === undefined) continue;
    const properties: Record<string, unknown> = {};
    if (tag.name === 'a') {
      const href = tag.attributes.href ?? '';
      if (isSafeHttps(href)) Object.assign(properties, { href, target: '_blank', rel: 'noreferrer' });
    }
    stack.push({ children: [], outputTag: tag.name === 'a' && properties.href === undefined ? 'span' : outputTag, properties, sourceTag: tag.name });
    if (tag.selfClosing) close(tag.name);
  }
  while (stack.length > 1) {
    const frame = stack.pop();
    if (frame !== undefined) append(h(frame.outputTag, frame.properties, frame.children));
  }
  const root = stack[0]!;
  return h(root.outputTag, root.properties, root.children) as VNode;
}

function markdownWithoutRawHtml(body: string): string {
  const source = body.slice(0, MAX_MARKUP_CHARS);
  let text = '';
  let index = 0;
  let operations = 0;
  let suppressed: 'script' | 'style' | null = null;
  while (index < source.length && operations < MAX_NODES) {
    operations += 1;
    if (source.startsWith('<!--', index)) {
      const commentEnd = source.indexOf('-->', index + 4);
      index = commentEnd < 0 ? source.length : commentEnd + 3;
      continue;
    }
    if (source[index] !== '<') {
      const next = source.indexOf('<', index);
      if (suppressed === null) text += source.slice(index, next < 0 ? source.length : next);
      index = next < 0 ? source.length : next;
      continue;
    }
    const tag = parseHtmlTag(source, index);
    if (tag === null) {
      if (suppressed === null) text += '<';
      index += 1;
      continue;
    }
    index = tag.end;
    if (suppressed !== null) {
      if (tag.closing && tag.name === suppressed) suppressed = null;
    } else if (!tag.closing && (tag.name === 'script' || tag.name === 'style')) {
      suppressed = tag.name;
    }
  }
  return text;
}

export function renderMarketplaceBody(body: string, format: MarketplaceBodyFormat): VNode {
  if (format === 'plain_text') {
    return h(
      'div',
      { class: 'marketplace-markdown marketplace-markdown--plain' },
      body.slice(0, MAX_MARKUP_CHARS).split(/\n{2,}/u).slice(0, MAX_NODES)
        .map((paragraph, index) => h('p', { key: `plain-${index}` }, paragraph)),
    ) as VNode;
  }
  if (format === 'html') return renderMarketplaceHtml(body);
  const lines = markdownWithoutRawHtml(body).replace(/\r\n/gu, '\n').split('\n');
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
