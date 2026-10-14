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
const AUTOLINK = /^<([A-Za-z][A-Za-z0-9+.-]{1,31}:[^\s<>]*)>/
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

/** CommonMark: a code span closes only at a backtick run of exactly the opening run's length. */
function closingRun(source: string, from: number, length: number): number {
  for (let at = source.indexOf('`', from); at !== -1;) {
    const run = runLength(source, at, '`')
    if (run === length) return at
    at = source.indexOf('`', at + run)
  }
  return -1
}

const unescape = (raw: string) => raw.replace(/\\([!-/:-@[-`{-~])/g, '$1')

interface LinkMatch { readonly label: string; readonly destination: string; readonly end: number }

/** `[label](destination "optional title")`; the label may hold brackets, escapes and code spans. Titles are dropped. */
function matchLink(source: string, open: number): LinkMatch | undefined {
  let depth = 0
  let close = open
  for (; close < source.length; close++) {
    const char = source[close]
    if (char === '\\') { close++; continue }
    if (char === '`') {
      const run = runLength(source, close, '`')
      const end = closingRun(source, close + run, run)
      close = (end === -1 ? close + run : end + run) - 1
      continue
    }
    if (char === '[') depth++
    else if (char === ']' && --depth === 0) break
  }
  if (close >= source.length || source[close + 1] !== '(') return undefined
  let at = close + 2
  while (source[at] === ' ') at++
  let destination: string
  if (source[at] === '<') {
    const end = source.indexOf('>', at)
    if (end === -1 || source.slice(at + 1, end).includes('<')) return undefined
    destination = unescape(source.slice(at + 1, end))
    at = end + 1
  } else {
    const start = at
    let parens = 0
    for (; at < source.length; at++) {
      const char = source[at]!
      if (char === '\\' && isEscapable(source[at + 1])) { at++; continue }
      if (/\s/u.test(char)) break
      if (char === '(') parens++
      else if (char === ')') { if (parens === 0) break; parens-- }
    }
    destination = unescape(source.slice(start, at))
  }
  while (source[at] === ' ') at++
  const quote = source[at]
  if (quote === '"' || quote === '\'' || quote === '(') {
    const end = quote === '(' ? ')' : quote
    let titleEnd = at + 1
    for (; titleEnd < source.length && source[titleEnd] !== end; titleEnd++) if (source[titleEnd] === '\\') titleEnd++
    if (titleEnd >= source.length) return undefined
    at = titleEnd + 1
    while (source[at] === ' ') at++
  }
  if (source[at] !== ')') return undefined
  return { label: source.slice(open + 1, close), destination, end: at + 1 }
}

function scan(source: string, allowLinks: boolean): Item[] {
  const items: Item[] = []
  let text = ''
  const flush = () => {
    if (text !== '') items.push({ _tag: 'Text', text })
    text = ''
  }
  let at = 0
  while (at < source.length) {
    const char = source[at]!
    if (char === '\\' && isEscapable(source[at + 1])) {
      text += source[at + 1]
      at += 2
      continue
    }
    if (char === '`') {
      const run = runLength(source, at, '`')
      const end = closingRun(source, at + run, run)
      if (end === -1) {
        // An unmatched run is literal backticks, never a code span.
        text += '`'.repeat(run)
        at += run
        continue
      }
      flush()
      const raw = source.slice(at + run, end)
      // One leading and one trailing space are stripped when both exist and the span is not only spaces.
      items.push({ _tag: 'Code', code: raw.length >= 2 && raw.startsWith(' ') && raw.endsWith(' ') && raw.trim() !== '' ? raw.slice(1, -1) : raw })
      at = end + run
      continue
    }
    if (char === '*' || char === '_') {
      const run = runLength(source, at, char)
      const before = source[at - 1], after = source[at + run]
      const leftFlanking = !isWhitespace(after) && (!isPunctuation(after) || isWhitespace(before) || isPunctuation(before))
      const rightFlanking = !isWhitespace(before) && (!isPunctuation(before) || isWhitespace(after) || isPunctuation(after))
      flush()
      items.push({
        _tag: 'Delimiter', char, length: run, count: run,
        // Intraword underscores (snake_case) never open or close emphasis.
        canOpen: char === '*' ? leftFlanking : leftFlanking && (!rightFlanking || isPunctuation(before)),
        canClose: char === '*' ? rightFlanking : rightFlanking && (!leftFlanking || isPunctuation(after)),
      })
      at += run
      continue
    }
    if (char === '<') {
      const autolink = AUTOLINK.exec(source.slice(at))
      if (autolink !== null) {
        flush()
        const url = autolink[1]!
        items.push(allowLinks ? { _tag: 'Link', href: safeLinkHref(url), children: [{ _tag: 'Text', text: url }] } : { _tag: 'Text', text: url })
        at += autolink[0].length
        continue
      }
    }
    const image = char === '!' && source[at + 1] === '['
    if (char === '[' || image) {
      const link = matchLink(source, image ? at + 1 : at)
      if (link !== undefined) {
        flush()
        const children = resolve(scan(link.label, false))
        // Images render their alt text only; a link nested in a link label keeps its label only.
        if (image || !allowLinks) items.push(...children)
        else items.push({ _tag: 'Link', href: safeLinkHref(link.destination), children })
        at = link.end
        continue
      }
    }
    text += char
    at++
  }
  flush()
  return items
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

/** CommonMark "process emphasis": each closer pairs with the nearest compatible opener, including the rule of three. */
function resolve(items: Item[]): InlineNode[] {
  for (let index = 0; index < items.length; index++) {
    const closer = items[index]!
    if (closer._tag !== 'Delimiter' || !closer.canClose || closer.count === 0) continue
    let openerIndex = index - 1
    for (; openerIndex >= 0; openerIndex--) {
      const opener = items[openerIndex]!
      if (opener._tag !== 'Delimiter' || opener.char !== closer.char || !opener.canOpen || opener.count === 0) continue
      const ruleOfThree = (opener.canClose || closer.canOpen) && (opener.length + closer.length) % 3 === 0 && !(opener.length % 3 === 0 && closer.length % 3 === 0)
      if (!ruleOfThree) break
    }
    if (openerIndex < 0) continue
    const opener = items[openerIndex] as Delimiter
    const used = opener.count >= 2 && closer.count >= 2 ? 2 : 1
    const children = merge(items.slice(openerIndex + 1, index).map(literal))
    opener.count -= used
    closer.count -= used
    items.splice(openerIndex + 1, index - openerIndex - 1, { _tag: used === 2 ? 'Strong' : 'Emphasis', children })
    let closerIndex = openerIndex + 2
    if (opener.count === 0) {
      items.splice(openerIndex, 1)
      closerIndex--
    }
    if (closer.count === 0) items.splice(closerIndex, 1)
    // Revisit this position: a closer with delimiters left can close again, otherwise the next item is here.
    index = closerIndex - 1
  }
  return merge(items.map(literal))
}

/** Line endings become spaces: the inline subset is single-line phrasing content. */
export function parseInlineMarkdown(text: string): readonly InlineNode[] {
  return resolve(scan(text.replace(/\r\n?|\n/g, ' '), true))
}

const plain = (nodes: readonly InlineNode[]): string => nodes.map(node => node._tag === 'Text' ? node.text : node._tag === 'Code' ? node.code : plain(node.children)).join('')

/** Markers stripped for `title`, `aria-label` and `textValue`: code, emphasis and link labels keep their text. */
export function toPlainText(text: string): string {
  return plain(parseInlineMarkdown(text)).replace(/\s+/g, ' ').trim()
}
