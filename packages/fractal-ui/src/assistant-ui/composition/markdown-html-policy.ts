/**
 * The HTML that Markdown may carry, modelled on what GitHub renders: a fixed set of elements and attributes, links
 * that only go to http(s), no comments, nothing that runs or embeds, and bounded nesting. Raw HTML is never handed to
 * the page. Allowed elements become ordinary syntax-tree nodes that React renders as elements, and everything else
 * becomes escaped text or disappears. The policy is data (`HTML_POLICY`) so other clients can apply the same rules.
 *
 * Threats and the rule that answers each, with tests in Markdown.unit.test.mjs:
 * - script, event handlers, `javascript:`/`data:`/`vbscript:` URLs: elements outside the list never render, attributes
 *   outside the list are dropped, `href` keeps only http(s) after entity decoding and whitespace removal.
 * - embedding and forms (iframe, object, embed, form, input, svg, math): dropped with their content.
 * - hidden text (`<!-- -->`, an unterminated comment): removed; an unterminated comment hides the rest of its block.
 * - layout and spoofing (style, class, id): no styling attribute survives.
 * - remote resources (img, picture, audio, video): images become the same deferred, consent-gated placeholder Markdown
 *   images use; media elements are dropped.
 * - resource exhaustion (thousands of nested quotes, lists, tags): nesting past `maxDepth` collapses to plain text.
 *
 * The allowlist governs raw HTML. Markdown syntax keeps the elements it defines: a task list item's checkbox (disabled,
 * as GitHub draws it) and the footnotes section.
 */

export const HTML_POLICY = {
  /** Element name → attributes it keeps. */
  elements: {
    a: ['href', 'title'], abbr: ['title'], b: [], blockquote: [], br: [], code: [], dd: [], del: [], details: ['open'],
    div: [], dl: [], dt: [], em: [], h1: [], h2: [], h3: [], h4: [], h5: [], h6: [], hr: [], i: [],
    img: ['src', 'alt', 'title', 'width', 'height'], ins: [], kbd: [], li: [], mark: [], ol: ['start'], p: [], pre: [],
    q: [], s: [], samp: [], small: [], span: [], strike: [], strong: [], sub: [], summary: [], sup: [],
    table: [], tbody: [], td: ['align', 'colspan', 'rowspan'], tfoot: [], th: ['align', 'colspan', 'rowspan'], thead: [],
    tr: [], tt: [], u: [], ul: [], var: [],
  } as Readonly<Record<string, readonly string[]>>,
  /** Elements whose content goes with them. */
  dropWithContent: ['applet', 'audio', 'button', 'canvas', 'embed', 'form', 'frame', 'frameset', 'head', 'iframe', 'input', 'math', 'noembed', 'noframes', 'noscript', 'object', 'option', 'plaintext', 'script', 'select', 'style', 'svg', 'template', 'textarea', 'title', 'video', 'xmp'] as readonly string[],
  /** Protocols a link may use. Anything else, relative links included, shows as its text. */
  linkProtocols: ['http:', 'https:'] as readonly string[],
  /** The one relative form kept: a Fractal document reference (`doc/name`, `doc/name@hash`), which a document host opens in place. */
  referenceLink: '^doc/(?!(?:.*/)?\\.\\.(?:/|@|$))[A-Za-z0-9._~-]+(?:/[A-Za-z0-9._~-]+)*(?:@[0-9a-f]{6,64})?$',
  /** Nesting deeper than this, of quotes, lists or HTML elements, shows as plain text. */
  maxDepth: 32,
} as const

const VOID = new Set(['br', 'hr', 'img', 'wbr', 'input', 'meta', 'link', 'base', 'source', 'area', 'col', 'embed', 'param', 'track'])
/** Real HTML element names; a tag-like word outside this set (`<T>`, `Array<string>`) stays text. */
const HTML_NAMES = new Set([...Object.keys(HTML_POLICY.elements), ...HTML_POLICY.dropWithContent, ...'address area article aside base bdi bdo body caption center cite col colgroup data datalist dfn dialog dir figcaption figure font footer header html label legend link main map marquee menu meta meter nav optgroup output param picture progress rp rt ruby search section slot source time track wbr'.split(' ')])
const FLOW = new Set(['root', 'blockquote', 'listItem', 'footnoteDefinition', 'details'])
const NESTING = new Set(['blockquote', 'list', 'listItem', 'htmlElement', 'details', 'emphasis', 'strong', 'delete', 'link', 'linkReference'])

interface Node { type: string; value?: string; url?: string; children?: Node[]; data?: { hName?: string; hProperties?: Record<string, string | boolean> } }
type Token = { readonly kind: 'text'; readonly value: string } | { readonly kind: 'open'; readonly name: string; readonly attributes: string; readonly selfClosing: boolean } | { readonly kind: 'close'; readonly name: string } | { readonly kind: 'comment'; readonly closed: boolean }

/** The named references HTML text commonly uses; any other name stays as written. */
const ENTITIES: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: '\u00a0', colon: ':', tab: '\t', newline: '\n', copy: '\u00a9', reg: '\u00ae', trade: '\u2122', hellip: '\u2026', mdash: '\u2014', ndash: '\u2013', lsquo: '\u2018', rsquo: '\u2019', ldquo: '\u201c', rdquo: '\u201d', laquo: '\u00ab', raquo: '\u00bb', middot: '\u00b7', bull: '\u2022', times: '\u00d7', divide: '\u00f7', deg: '\u00b0', plusmn: '\u00b1', larr: '\u2190', rarr: '\u2192', uarr: '\u2191', darr: '\u2193', harr: '\u2194', euro: '\u20ac', pound: '\u00a3', yen: '\u00a5', cent: '\u00a2', sect: '\u00a7', para: '\u00b6', check: '\u2713', ensp: '\u2002', emsp: '\u2003', thinsp: '\u2009', zwj: '\u200d', zwnj: '\u200c' }
export const decodeEntities = (text: string): string => text.replace(/&(?:#x([0-9a-f]{1,6})|#(\d{1,7})|([a-z]+));?/gi, (whole, hex: string | undefined, decimal: string | undefined, name: string | undefined) => {
  const point = hex !== undefined ? parseInt(hex, 16) : decimal !== undefined ? parseInt(decimal, 10) : undefined
  if (point !== undefined) return point > 0 && point <= 0x10ffff && (point < 0xd800 || point > 0xdfff) ? String.fromCodePoint(point) : '\ufffd'
  return ENTITIES[name!.toLowerCase()] ?? whole
})

/**
 * A URL a link may open, or `undefined`. Control characters and whitespace that browsers ignore cannot hide a scheme.
 * `raw` is already entity-decoded (the Markdown parser decodes destinations, `properties` decodes attributes): decoding
 * again would change a destination such as `?q=&amp;colon;`.
 */
const REFERENCE_LINK = new RegExp(HTML_POLICY.referenceLink)
export function safeLinkHref(raw: string): string | undefined {
  const cleaned = raw.replace(/[\u0000-\u0020\u007f-\u009f]/g, '')
  if (REFERENCE_LINK.test(cleaned)) return cleaned
  if (!/^[a-z][a-z0-9+.-]*:/i.test(cleaned)) return undefined
  try {
    const url = new URL(cleaned)
    return HTML_POLICY.linkProtocols.includes(url.protocol) ? cleaned : undefined
  } catch { return undefined }
}

const TAG_MAX = 4096
/**
 * Where the tag starting at `at` ends (its `>` outside quotes), or -1 when it is not a tag: another `<` outside quotes
 * comes first, or it runs past `TAG_MAX`. The bound keeps tokenizing linear however malformed the input.
 */
export function tagEnd(html: string, at: number): number {
  let quote: string | undefined
  for (let index = at + 1; index < html.length && index - at < TAG_MAX; index++) {
    const character = html[index]
    if (quote !== undefined) { if (character === quote) quote = undefined; continue }
    if (character === '"' || character === "'") quote = character
    else if (character === '>') return index
    else if (character === '<') return -1
  }
  return -1
}
/** One tag, matched against its candidate text only. */
const TAG = /^<(\/?)([A-Za-z][A-Za-z0-9-]*)((?:\s+[^\s"'>/=]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s"'=<>]+))?)*)\s*(\/?)>$/
function tokenize(html: string): Token[] {
  const tokens: Token[] = []
  let text = 0
  let at = html.indexOf('<')
  const flush = (end: number) => { if (end > text) tokens.push({ kind: 'text', value: html.slice(text, end) }) }
  while (at !== -1) {
    if (html.startsWith('<!--', at)) {
      flush(at)
      const end = html.indexOf('-->', at + 4)
      tokens.push({ kind: 'comment', closed: end !== -1 })
      if (end === -1) return tokens
      text = end + 3
      at = html.indexOf('<', text)
      continue
    }
    const close = tagEnd(html, at)
    if (close === -1) { at = html.indexOf('<', at + 1); continue }
    const match = TAG.exec(html.slice(at, close + 1))
    const name = match?.[2]?.toLowerCase()
    if (match !== null && name !== undefined && HTML_NAMES.has(name)) {
      flush(at)
      tokens.push(match[1] === '/' ? { kind: 'close', name } : { kind: 'open', name, attributes: match[3] ?? '', selfClosing: match[4] === '/' })
      text = close + 1
    }
    at = html.indexOf('<', close + 1)
  }
  flush(html.length)
  return tokens
}

function properties(name: string, raw: string): Record<string, string | boolean> {
  const allowed = HTML_POLICY.elements[name] ?? []
  const kept: Record<string, string | boolean> = {}
  for (const match of raw.matchAll(/([^\s"'>/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>]+)))?/g)) {
    const attribute = match[1]!.toLowerCase()
    if (!allowed.includes(attribute)) continue
    const value = decodeEntities(match[2] ?? match[3] ?? match[4] ?? '')
    if (attribute === 'href') { const href = safeLinkHref(value); if (href !== undefined) kept.href = href; continue }
    if (attribute === 'src') { const src = safeLinkHref(value); if (src !== undefined) kept.src = src; continue }
    if (attribute === 'open') { kept.open = true; continue }
    // Syntax-tree property names are the DOM's (`colSpan`), not the attribute's.
    if (['colspan', 'rowspan', 'start', 'width', 'height'].includes(attribute)) { if (/^\d{1,4}$/.test(value)) kept[attribute === 'colspan' ? 'colSpan' : attribute === 'rowspan' ? 'rowSpan' : attribute] = value; continue }
    if (attribute === 'align') { if (/^(?:left|center|right)$/i.test(value)) kept.align = value.toLowerCase(); continue }
    kept[attribute] = value
  }
  return kept
}

const element = (name: string, raw: string, children: Node[] = []): Node => ({ type: name === 'details' ? 'details' : 'htmlElement', children, data: { hName: name, hProperties: properties(name, raw) } })

/**
 * Applies `HTML_POLICY` to a Markdown syntax tree: raw HTML becomes allowed elements or text, links outside the
 * allowed protocols become their text, and nesting past `maxDepth` becomes plain text.
 */
export function remarkHtmlPolicy() {
  const rebuild = (parent: Node): void => {
    if (parent.children === undefined) return
    const flow = FLOW.has(parent.type)
    const root: Node[] = []
    const stack: { readonly name: string; readonly node: Node }[] = []
    // Open elements by name, so a close tag with no opener costs nothing (no stack scan).
    const opened = new Map<string, number>()
    let dropping: { readonly name: string; depth: number } | undefined
    let hidden = false
    const target = (): Node[] => stack.at(-1)?.node.children ?? root
    const atFlow = (): boolean => stack.length === 0 ? flow : STRUCTURAL.has(stack.at(-1)!.name)
    const append = (node: Node): void => {
      const into = target()
      // Text that lands directly in a block container reads as a paragraph.
      if ((node.type === 'text' || node.type === 'inlineCode' || node.type === 'htmlElement' && !blockName(node)) && atFlow()) {
        const last = into.at(-1)
        if (last?.type === 'paragraph' && last.data?.hName === undefined && (last as { loose?: boolean }).loose === true) last.children!.push(node)
        else into.push({ type: 'paragraph', children: [node], ...{ loose: true } } as Node)
        return
      }
      into.push(node)
    }
    for (const child of parent.children) {
      if (hidden) continue
      if (child.type !== 'html') {
        if (dropping !== undefined) continue
        if (child.type === 'link' && safeLinkHref(child.url ?? '') === undefined) { rebuild(child); for (const inner of child.children ?? []) append(inner); continue }
        if (child.type === 'definition' && safeLinkHref(child.url ?? '') === undefined) child.url = ''
        rebuild(child)
        append(child)
        continue
      }
      for (const token of tokenize(child.value ?? '')) {
        if (hidden) break
        if (token.kind === 'comment') { if (!token.closed) hidden = true; continue }
        if (dropping !== undefined) {
          // Nested elements of the same name keep the outer one dropping until its own close.
          if (token.kind === 'open' && token.name === dropping.name && !token.selfClosing) dropping.depth += 1
          if (token.kind === 'close' && token.name === dropping.name && --dropping.depth === 0) dropping = undefined
          continue
        }
        // Whitespace between blocks is layout; between inline elements it is a space the reader sees.
        if (token.kind === 'text') { const value = decodeEntities(token.value); if (value.trim() !== '' || !atFlow()) append({ type: 'text', value }); continue }
        if (token.kind === 'open' && HTML_POLICY.dropWithContent.includes(token.name)) { if (!token.selfClosing && !VOID.has(token.name)) dropping = { name: token.name, depth: 1 }; continue }
        if (!(token.name in HTML_POLICY.elements)) continue
        // The synthetic code child already gives raw pre text the shape CodeFence consumes.
        // An explicit <code> is that same holder, not another nested code element.
        if (token.kind === 'open' && token.name === 'code' && stack.at(-1)?.name === 'pre') {
          if (!token.selfClosing) { stack.push({ name: 'code', node: stack.at(-1)!.node }); opened.set('code', (opened.get('code') ?? 0) + 1) }
          continue
        }
        if (token.kind === 'open') {
          const node = element(token.name, token.attributes)
          if (token.name === 'a' && node.data?.hProperties?.href === undefined) continue
          if (token.name === 'img' && node.data?.hProperties?.src === undefined) { const alt = node.data?.hProperties?.alt; if (typeof alt === 'string' && alt !== '') append({ type: 'text', value: alt }); continue }
          // Past the depth limit an element adds no nesting: its tag goes and its content stays.
          if (stack.length >= HTML_POLICY.maxDepth) continue
          append(node)
          // A code block's text goes in a `code` child, the shape the code renderer reads.
          const holder = token.name === 'pre' ? (node.children!.push({ type: 'htmlElement', children: [], data: { hName: 'code', hProperties: {} } }), node.children![0]!) : node
          if (!token.selfClosing && !VOID.has(token.name)) { stack.push({ name: token.name, node: holder }); opened.set(token.name, (opened.get(token.name) ?? 0) + 1) }
          continue
        }
        if ((opened.get(token.name) ?? 0) === 0) continue
        while (stack.length > 0) {
          const closed = stack.pop()!
          opened.set(closed.name, opened.get(closed.name)! - 1)
          if (closed.name === token.name) break
        }
      }
    }
    parent.children = root.map(node => { delete (node as { loose?: boolean }).loose; return node })
  }
  // Bounding Markdown nesting first keeps `rebuild`'s recursion shallow; the second pass bounds what HTML added.
  return (tree: Node) => { capDepth(tree, 0); rebuild(tree); capDepth(tree, 0) }
}

export interface MarkdownParsedTree { tree?: Node }

/** Retain the policy-filtered mdast, not another parse or a source-based approximation, for render failures. */
export function remarkCaptureTree(capture: MarkdownParsedTree) {
  return (tree: Node) => { capture.tree = tree }
}

/** Escaped React text is derived only from policy-approved text/code nodes; HTML can never be a fallback value. */
export function markdownTreeText(tree: Node): string {
  const values: string[] = []
  const pending: (Node | string)[] = [tree]
  while (pending.length > 0) {
    const node = pending.pop()!
    if (typeof node === 'string') { values.push(node); continue }
    if (node.type === 'html') continue
    if (node.value !== undefined) values.push(node.value)
    if (node.children !== undefined) {
      if (/^(?:paragraph|heading|blockquote|listItem|code|tableRow)$/.test(node.type)) pending.push('\n')
      for (let index = node.children.length - 1; index >= 0; index--) pending.push(node.children[index]!)
    }
  }
  return values.join('').trimEnd()
}

/**
 * Fail-closed HTML-only fallback, deliberately NOT a second Markdown parser. Delimiter/image floods cannot use
 * CommonMark safely (nested images remain quadratic with attention disabled). Render errors scrub approved mdast
 * text through this same function. These rare views may hide literal code and the rest of the message.
 *
 * Dropped-element closer: none is accepted, an empty subset of normal closers; even an exact closer may be code.
 * Comment end: none is accepted; a malformed inline opener may not be a normal comment and can contain a dropped
 * opener before `-->`. Hiding to EOF avoids releasing its suffix earlier than the normal policy.
 * CDATA/declaration/processing-instruction end: none is accepted; their embedded tags cannot release hidden text.
 * Tag-name end: ASCII names are read once; recognizing extra malformed dropped openers only hides more.
 * Attribute quote/end: only matching ASCII quotes release quoted spans; unterminated attributes hide to EOF.
 * Nested `<` outside quotes: reprocess it, so an earlier malformed ordinary tag cannot swallow a dropped opener.
 * Every cursor move is forward; each character belongs to one ordinary-text, name, or attribute scan: O(n).
 */
export function filteredMarkdownText(source: string): string {
  const values: string[] = []
  let plain = 0
  let at = 0
  while (at < source.length) {
    if (source.charCodeAt(at) !== 60) { at++; continue }
    const start = at++
    if (source[at] === '!' || source[at] === '?') {
      values.push(source.slice(plain, start))
      return values.join('')
    }
    const closing = source[at] === '/'
    if (closing) at++
    const nameStart = at
    const first = source.charCodeAt(at)
    if (!((first >= 65 && first <= 90) || (first >= 97 && first <= 122))) continue
    while (at < source.length) {
      const code = source.charCodeAt(at)
      if (!((code >= 65 && code <= 90) || (code >= 97 && code <= 122) || (code >= 48 && code <= 57) || code === 58 || code === 45)) break
      at++
    }
    const name = source.slice(nameStart, at).toLowerCase()
    values.push(source.slice(plain, start))
    if (!closing && HTML_POLICY.dropWithContent.includes(name)) return values.join('')
    let quote: string | undefined
    while (at < source.length) {
      const character = source[at]!
      if (quote !== undefined) { if (character === quote) quote = undefined }
      else if (character === '"' || character === "'") quote = character
      else if (character === '<') break
      else if (character === '>') { at++; break }
      at++
    }
    plain = at
  }
  values.push(source.slice(plain))
  return values.join('')
}

/** Containers whose direct text is layout, not words: whitespace there is dropped and words become paragraphs. */
const STRUCTURAL = new Set(['blockquote', 'details', 'dl', 'ol', 'table', 'tbody', 'tfoot', 'thead', 'tr', 'ul'])
const blockName = (node: Node): boolean => /^(?:address|blockquote|details|div|dl|h[1-6]|hr|ol|p|pre|summary|table|ul)$/.test(node.data?.hName ?? '')

/** The words of a subtree, read without recursion: the subtree may be thousands of levels deep. */
function plainText(root: Node): string {
  const words: string[] = []
  const pending: Node[] = [root]
  while (pending.length > 0) {
    const node = pending.pop()!
    if (node.value !== undefined) words.push(node.value)
    else if (node.children !== undefined) for (let index = node.children.length - 1; index >= 0; index--) pending.push(node.children[index]!)
  }
  return words.join(' ').replace(/\s+/g, ' ').trim()
}

function capDepth(node: Node, depth: number): void {
  if (node.children === undefined) return
  node.children = node.children.map(child => {
    if (!NESTING.has(child.type) || depth + 1 <= HTML_POLICY.maxDepth) { capDepth(child, NESTING.has(child.type) ? depth + 1 : depth); return child }
    const text = { type: 'text', value: plainText(child) }
    if (node.type === 'list') return { type: 'listItem', children: [{ type: 'paragraph', children: [text] }] }
    return FLOW.has(node.type) ? { type: 'paragraph', children: [text] } : text
  })
}
