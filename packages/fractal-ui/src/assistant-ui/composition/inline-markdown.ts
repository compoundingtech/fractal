/**
 * The inline Markdown subset every one-line agent or user text surface renders: code spans, strong, emphasis
 * and links (inline and `<scheme:…>` autolinks). It follows CommonMark for backtick-run matching, backslash
 * escapes and emphasis delimiter runs. It never parses HTML: `<img onerror>` stays literal text.
 * Pure and dependency-free, so the runtime-free shell entry can carry it (scripts/check-shell-imports.mjs).
 */
export type InlineNode =
  | { readonly _tag: 'Text'; readonly text: string }
  | { readonly _tag: 'Code'; readonly code: string }
  | { readonly _tag: 'Strong'; readonly children: readonly InlineNode[] }
  | { readonly _tag: 'Emphasis'; readonly children: readonly InlineNode[] }
  /** `href` is absent when the destination is empty or uses an unsafe protocol; the label still renders. */
  | { readonly _tag: 'Link'; readonly href: string | undefined; readonly children: readonly InlineNode[] }

interface Delimiter { readonly _tag: 'Delimiter'; readonly char: '*' | '_'; readonly length: number; count: number; readonly canOpen: boolean; readonly canClose: boolean }
type Item = InlineNode | Delimiter

const ASCII_PUNCTUATION = /^[!-/:-@[-`{-~]$/
const SAFE_PROTOCOL = /^(?:https?|ircs?|mailto|xmpp)$/i
const AUTOLINK = /<([A-Za-z][A-Za-z0-9+.-]{1,31}:[^\s<>]*)>/y
const isWhitespace = (char: string | undefined) => char === undefined || /\s/u.test(char)
const isPunctuation = (char: string | undefined) => char !== undefined && /[\p{P}\p{S}]/u.test(char)
const isEscapable = (char: string | undefined) => char !== undefined && ASCII_PUNCTUATION.test(char)

/** The block renderer's rule (react-markdown `defaultUrlTransform`): relative URLs and http(s), irc(s), mailto and xmpp only. */
export function safeLinkHref(url: string): string | undefined {
  if (url === '') return undefined
  const colon = url.indexOf(':')
  const slash = url.indexOf('/'), query = url.indexOf('?'), fragment = url.indexOf('#')
  const relative = colon === -1 || (slash !== -1 && colon > slash) || (query !== -1 && colon > query) || (fragment !== -1 && colon > fragment)
  return relative || SAFE_PROTOCOL.test(url.slice(0, colon)) ? url : undefined
}

const runLength = (source: string, at: number, char: string) => {
  let end = at
  while (source[end] === char) end++
  return end - at
}

/** One suffix table, including partial runs after escapes; no repeated indexOf suffix searches. */
const codeMatches = (source: string): ReadonlyMap<number, number> => {
  const ends = new Map<number, number>()
  const next = new Map<number, number>()
  for (let at = source.length - 1; at >= 0; at--) {
    if (source[at] !== '`') continue
    const end = at + 1
    while (at > 0 && source[at - 1] === '`') at--
    for (let start = at; start < end; start++) ends.set(start, next.get(end - start) ?? -1)
    next.set(end - at, at)
  }
  return ends
}

const unescape = (raw: string) => raw.replace(/\\([!-/:-@[-`{-~])/g, '$1')

interface LinkMatch { readonly close: number; readonly start: number; readonly destinationEnd: number; readonly angle: boolean; readonly end: number }

/**
 * Match brackets once, from right to left. Each suffix records its first unmatched
 * closing bracket; an opening bracket skips its complete nested label in O(1).
 * Computing suffixes independently preserves labels even after skipped link destinations.
 */
const bracketMatches = (source: string, codeEnds: ReadonlyMap<number, number>): ReadonlyMap<number, number> => {
  const closes = new Map<number, number>()
  const nextClose = new Int32Array(source.length + 2).fill(-1)
  let codeRunLength = 0
  for (let at = source.length - 1; at >= 0; at--) {
    const char = source[at]
    codeRunLength = char === '`' ? codeRunLength + 1 : 0
    if (char === '\\') nextClose[at] = nextClose[at + 2]!
    else if (char === '`') {
      const end = at + codeRunLength
      const close = codeEnds.get(at) ?? -1
      nextClose[at] = nextClose[close === -1 ? end : close + end - at]!
    } else if (char === ']') nextClose[at] = at
    else if (char === '[') {
      const close = nextClose[at + 1]!
      if (close !== -1) closes.set(at, close)
      nextClose[at] = close === -1 ? -1 : nextClose[close + 1]!
    } else nextClose[at] = nextClose[at + 1]!
  }
  return closes
}

interface DestinationMatches {
  readonly ends: Int32Array
  readonly spaces: Int32Array
  readonly angles: Int32Array
  readonly titles: Record<'"' | "'" | ')', Int32Array>
}

/** Suffix transitions share failed scans as well as balanced parentheses and escaped title ends. */
const destinationMatches = (source: string): DestinationMatches => {
  const size = source.length
  const ends = new Int32Array(size + 2).fill(size)
  const spaces = new Int32Array(size + 2).fill(size)
  const angles = new Int32Array(size + 2).fill(-1)
  const titles = { '"': new Int32Array(size + 2).fill(-1), "'": new Int32Array(size + 2).fill(-1), ')': new Int32Array(size + 2).fill(-1) }
  for (let at = size - 1; at >= 0; at--) {
    const char = source[at]!
    spaces[at] = char === ' ' ? spaces[at + 1]! : at
    angles[at] = char === '>' ? at : char === '<' ? -1 : angles[at + 1]!
    for (const quote of ['"', "'", ')'] as const) {
      const matches = titles[quote]
      matches[at] = char === quote ? at : matches[at + (char === '\\' ? 2 : 1)]!
    }
    if (char === '\\' && isEscapable(source[at + 1])) ends[at] = ends[at + 2]!
    else if (/\s/u.test(char) || char === ')') ends[at] = at
    else if (char === '(') {
      const close = ends[at + 1]!
      ends[at] = source[close] === ')' ? ends[close + 1]! : close
    } else ends[at] = ends[at + 1]!
  }
  return { ends, spaces, angles, titles }
}

/** `[label](destination "optional title")`; the label may hold brackets, escapes and code spans. Titles are dropped. */
function matchLink(source: string, open: number, closes: ReadonlyMap<number, number>, matches: DestinationMatches): LinkMatch | undefined {
  const close = closes.get(open)
  if (close === undefined || source[close + 1] !== '(') return undefined
  let at = matches.spaces[close + 2]!
  const start = at
  let destinationEnd: number
  const angle = source[at] === '<'
  if (angle) {
    destinationEnd = matches.angles[at + 1]!
    if (destinationEnd === -1) return undefined
    at = destinationEnd + 1
  } else {
    destinationEnd = matches.ends[at]!
    at = destinationEnd
  }
  at = matches.spaces[at]!
  const quote = source[at]
  if (quote === '"' || quote === '\'' || quote === '(') {
    const titleEnd = matches.titles[quote === '(' ? ')' : quote][at + 1]!
    if (titleEnd === -1) return undefined
    at = matches.spaces[titleEnd + 1]!
  }
  if (source[at] !== ')') return undefined
  return {
    close, start, destinationEnd, angle,
    end: at + 1,
  }
}

function scan(source: string): InlineNode[] {
  const codes = codeMatches(source)
  let closes: ReadonlyMap<number, number> | undefined
  let destinations: DestinationMatches | undefined
  const links = new Map<number, LinkMatch>()
  const openers: number[] = []
  let inactiveThrough = -1
  const images = new Set<number>()
  const autolinks = new Map<number, { readonly url: string; readonly end: number }>()
  // CommonMark active brackets: a successful inner link deactivates all earlier
  // link openers. Every opener is pushed/popped once, including failed outer links.
  for (let at = 0; at < source.length;) {
    const char = source[at]!
    if (char === '\\' && isEscapable(source[at + 1])) { at += 2; continue }
    if (char === '`') {
      const run = runLength(source, at, '`'), end = codes.get(at) ?? -1
      at = end === -1 ? at + run : end + run
      continue
    }
    if (char === '<') {
      AUTOLINK.lastIndex = at
      const match = AUTOLINK.exec(source)
      if (match !== null) {
        autolinks.set(at, { url: match[1]!, end: AUTOLINK.lastIndex })
        inactiveThrough = at
        at = AUTOLINK.lastIndex
        continue
      }
    }
    if (char === '!' && source[at + 1] === '[') images.add(at + 1)
    if (char === '[') openers.push(at)
    else if (char === ']') {
      closes ??= bracketMatches(source, codes)
      const open = openers.pop()
      if (open !== undefined && (images.has(open) || open > inactiveThrough) && closes.get(open) === at && source[at + 1] === '(') {
        destinations ??= destinationMatches(source)
        const link = matchLink(source, open, closes, destinations)
        if (link !== undefined) {
          links.set(open, link)
          // Images retain their alt-label behavior; they do not deactivate links.
          if (!images.has(open)) inactiveThrough = at
          at = link.end
          continue
        }
      }
    }
    at++
  }
  return scanRanges(source, codes, links, autolinks, images)
}

/** Frames use offsets into the one source and share all lookup tables. Image
 * groups are transparent ropes until the final expansion, never copied per level. */
function scanRanges(source: string, codes: ReadonlyMap<number, number>, links: ReadonlyMap<number, LinkMatch>, autolinks: ReadonlyMap<number, { readonly url: string; readonly end: number }>, images: ReadonlySet<number>): InlineNode[] {
  interface Frame { readonly from: number; readonly to: number; readonly allowLinks: boolean; readonly link?: LinkMatch; readonly image?: boolean; readonly items: Item[]; text: string; at: number }
  const groups = new Set<InlineNode>()
  const frames: Frame[] = [{ from: 0, to: source.length, allowLinks: true, items: [], text: '', at: 0 }]
  let result: InlineNode[] = []
  while (frames.length > 0) {
    const frame = frames[frames.length - 1]!
    const { from, to, allowLinks, items } = frame
    let { at, text } = frame
    const flush = () => {
      if (text !== '') items.push({ _tag: 'Text', text })
      text = ''
    }
    if (at >= to) {
      flush()
      const children = resolve(items)
      frames.pop()
      const parent = frames[frames.length - 1]
      if (parent === undefined) { result = children; break }
      const link = frame.link!
      const transparent = frame.image || !parent.allowLinks
      const destination = transparent ? '' : unescape(source.slice(link.start + (link.angle ? 1 : 0), link.destinationEnd))
      const node: InlineNode = { _tag: 'Link', href: safeLinkHref(destination), children }
      if (transparent) groups.add(node)
      parent.items.push(node)
      continue
    }
    // Save the cursor even when a token branch continues this loop.
    const advance = (next: number) => { frame.at = next; frame.text = text }
    const char = source[at]!
    if (char === '\\' && at + 1 < to && isEscapable(source[at + 1])) {
      text += source[at + 1]
      advance(at + 2)
      continue
    }
    if (char === '`') {
      const run = runLength(source, at, '`')
      const match = codes.get(at) ?? -1
      const end = match < to ? match : -1
      if (end === -1) {
        // An unmatched run is literal backticks, never a code span.
        text += '`'.repeat(run)
        advance(at + run)
        continue
      }
      flush()
      const raw = source.slice(at + run, end)
      // One leading and one trailing space are stripped when both exist and the span is not only spaces.
      items.push({ _tag: 'Code', code: raw.length >= 2 && raw.startsWith(' ') && raw.endsWith(' ') && raw.trim() !== '' ? raw.slice(1, -1) : raw })
      advance(end + run)
      continue
    }
    if (char === '*' || char === '_') {
      const run = runLength(source, at, char)
      const before = at === from ? undefined : source[at - 1], after = at + run >= to ? undefined : source[at + run]
      const leftFlanking = !isWhitespace(after) && (!isPunctuation(after) || isWhitespace(before) || isPunctuation(before))
      const rightFlanking = !isWhitespace(before) && (!isPunctuation(before) || isWhitespace(after) || isPunctuation(after))
      flush()
      items.push({
        _tag: 'Delimiter', char, length: run, count: run,
        // Intraword underscores (snake_case) never open or close emphasis.
        canOpen: char === '*' ? leftFlanking : leftFlanking && (!rightFlanking || isPunctuation(before)),
        canClose: char === '*' ? rightFlanking : rightFlanking && (!leftFlanking || isPunctuation(after)),
      })
      advance(at + run)
      continue
    }
    if (char === '<') {
      const autolink = autolinks.get(at)
      if (autolink !== undefined && autolink.end <= to) {
        flush()
        const { url } = autolink
        items.push(allowLinks ? { _tag: 'Link', href: safeLinkHref(url), children: [{ _tag: 'Text', text: url }] } : { _tag: 'Text', text: url })
        advance(autolink.end)
        continue
      }
    }
    const image = char === '!' && images.has(at + 1)
    if (char === '[' || image) {
      const link = links.get(image ? at + 1 : at)
      if (link !== undefined && link.end <= to) {
        flush()
        advance(link.end)
        const start = (image ? at + 1 : at) + 1
        frames.push({ from: start, to: link.close, allowLinks: false, link, image, items: [], text: '', at: start })
        continue
      }
    }
    text += char
    advance(at + 1)
  }
  return groups.size === 0 ? result : expandGroups(result, groups)
}

/** Remove only private image/label ropes, preserving the public AST and text merging. */
function expandGroups(nodes: readonly InlineNode[], groups: ReadonlySet<InlineNode>): InlineNode[] {
  const output: InlineNode[] = []
  const frames: { readonly input: readonly InlineNode[]; readonly output: InlineNode[]; at: number }[] = [{ input: nodes, output, at: 0 }]
  while (frames.length > 0) {
    const frame = frames[frames.length - 1]!
    const node = frame.input[frame.at++]
    if (node === undefined) { frames.pop(); continue }
    if (node._tag === 'Text' || node._tag === 'Code') {
      const previous = frame.output[frame.output.length - 1]
      if (node._tag === 'Text' && previous?._tag === 'Text') frame.output[frame.output.length - 1] = { _tag: 'Text', text: previous.text + node.text }
      else frame.output.push(node)
    } else if (groups.has(node)) frames.push({ input: node.children, output: frame.output, at: 0 })
    else {
      const children: InlineNode[] = []
      frame.output.push({ ...node, children })
      frames.push({ input: node.children, output: children, at: 0 })
    }
  }
  return output
}

const literal = (item: Item): InlineNode => item._tag === 'Delimiter' ? { _tag: 'Text', text: item.char.repeat(item.count) } : item
function merge(items: readonly InlineNode[]): InlineNode[] {
  const merged: InlineNode[] = []
  for (const item of items) {
    const previous = merged[merged.length - 1]
    if (item._tag === 'Text' && item.text === '') continue
    if (item._tag === 'Text' && previous?._tag === 'Text') merged[merged.length - 1] = { _tag: 'Text', text: previous.text + item.text }
    else merged.push(item)
  }
  return merged
}

interface LinkedItem {
  readonly item: Item
  readonly index: number
  previous: LinkedItem | undefined
  next: LinkedItem | undefined
  previousDelimiter: LinkedItem | undefined
  nextDelimiter: LinkedItem | undefined
  activeDelimiter: boolean
}

/** CommonMark process-emphasis: failed searches have a per-type/rule-of-three lower bound. */
function resolve(items: Item[]): InlineNode[] {
  let first: LinkedItem | undefined
  let last: LinkedItem | undefined
  let firstDelimiter: LinkedItem | undefined
  let lastDelimiter: LinkedItem | undefined
  for (let index = 0; index < items.length; index++) {
    const node: LinkedItem = { item: items[index]!, index, previous: last, next: undefined, previousDelimiter: undefined, nextDelimiter: undefined, activeDelimiter: items[index]!._tag === 'Delimiter' }
    if (last !== undefined) last.next = node
    else first = node
    last = node
    if (node.item._tag === 'Delimiter') {
      node.previousDelimiter = lastDelimiter
      if (lastDelimiter !== undefined) lastDelimiter.nextDelimiter = node
      else firstDelimiter = node
      lastDelimiter = node
    }
  }
  const removeDelimiter = (node: LinkedItem) => {
    if (!node.activeDelimiter) return
    node.activeDelimiter = false
    if (node.previousDelimiter !== undefined) node.previousDelimiter.nextDelimiter = node.nextDelimiter
    if (node.nextDelimiter !== undefined) node.nextDelimiter.previousDelimiter = node.previousDelimiter
  }
  const removeItem = (node: LinkedItem) => {
    if (node.previous !== undefined) node.previous.next = node.next
    else first = node.next
    if (node.next !== undefined) node.next.previous = node.previous
  }
  // Six classes per character: whether a closer can open, and its original run length modulo three.
  const bottoms = new Int32Array(12).fill(-1)
  let current = firstDelimiter
  while (current !== undefined) {
    const closer = current.item
    if (closer._tag !== 'Delimiter' || !closer.canClose) { current = current.nextDelimiter; continue }
    const key = (closer.char === '*' ? 0 : 6) + (closer.canOpen ? 3 : 0) + closer.length % 3
    let candidate = current.previousDelimiter
    while (candidate !== undefined && candidate.index > bottoms[key]!) {
      const opener = candidate.item
      if (opener._tag === 'Delimiter' && opener.char === closer.char && opener.canOpen) {
        const ruleOfThree = (opener.canClose || closer.canOpen) && (opener.length + closer.length) % 3 === 0 && !(opener.length % 3 === 0 && closer.length % 3 === 0)
        if (!ruleOfThree) break
      }
      candidate = candidate.previousDelimiter
    }
    if (candidate === undefined || candidate.index <= bottoms[key]!) {
      bottoms[key] = current.previousDelimiter?.index ?? -1
      const next = current.nextDelimiter
      if (!closer.canOpen) removeDelimiter(current)
      current = next
      continue
    }
    const opener = candidate.item
    if (opener._tag !== 'Delimiter') continue
    const used = opener.count >= 2 && closer.count >= 2 ? 2 : 1
    const children: InlineNode[] = []
    for (let child = candidate.next; child !== undefined && child !== current; child = child.next) {
      children.push(literal(child.item))
      if (child.item._tag === 'Delimiter') removeDelimiter(child)
    }
    const wrapped: LinkedItem = {
      item: { _tag: used === 2 ? 'Strong' : 'Emphasis', children: merge(children) },
      index: candidate.index, previous: candidate, next: current,
      previousDelimiter: undefined, nextDelimiter: undefined, activeDelimiter: false,
    }
    candidate.next = wrapped
    current.previous = wrapped
    opener.count -= used
    closer.count -= used
    if (opener.count === 0) { removeDelimiter(candidate); removeItem(candidate) }
    if (closer.count === 0) {
      const next = current.nextDelimiter
      removeDelimiter(current)
      removeItem(current)
      current = next
    }
  }
  const nodes: InlineNode[] = []
  for (let node = first; node !== undefined; node = node.next) nodes.push(literal(node.item))
  return merge(nodes)
}

/** Line endings become spaces: the inline subset is single-line phrasing content. */
export function parseInlineMarkdown(text: string): readonly InlineNode[] {
  return scan(text.replace(/\r\n?|\n/g, ' '))
}

/** Iterative traversal, with one final join even for arbitrarily deep emphasis. */
export const inlineNodesText = (nodes: readonly InlineNode[]): string => {
  const parts: string[] = []
  const stack: { readonly nodes: readonly InlineNode[]; at: number }[] = [{ nodes, at: 0 }]
  while (stack.length > 0) {
    const frame = stack[stack.length - 1]!
    const node = frame.nodes[frame.at++]
    if (node === undefined) { stack.pop(); continue }
    if (node._tag === 'Text') parts.push(node.text)
    else if (node._tag === 'Code') parts.push(node.code)
    else stack.push({ nodes: node.children, at: 0 })
  }
  return parts.join('')
}

/** Markers stripped for `title`, `aria-label` and `textValue`: code, emphasis and link labels keep their text. */
export function toPlainText(text: string): string {
  return inlineNodesText(parseInlineMarkdown(text)).replace(/\s+/g, ' ').trim()
}
