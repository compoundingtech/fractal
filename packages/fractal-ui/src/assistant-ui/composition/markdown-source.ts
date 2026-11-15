import { HTML_POLICY, tagEnd } from './markdown-html-policy'
/**
 * Source-level helpers for the kit's Markdown: where a document may be cut into independently parsed blocks, and the
 * remark steps every surface shares. Nothing here imports a parser, so a later parser swap keeps the block contract.
 */

const LIST_LINE = /^ {0,3}(?:[-*+]|\d{1,9}[.)])[ \t]/
const FENCE_OPEN = /^( {0,3})(`{3,}|~{3,})(.*)$/
/** Definitions resolve across the whole document, also from inside quotes and list items. A task item may match: harmless, it only skips the cut. */
const DEFINITION = /^[ \t>]*(?:(?:[-*+]|\d{1,9}[.)])[ \t]+)*\[(?:\\[\s\S]|[^\[\]\\])+\]:/m
const RAW_HTML_OPEN = /^ {0,3}<(script|style|pre|textarea)(?=[\s>]|$)/i
const RAW_DECLARATION = /^ {0,3}<(?:\?|![A-Za-z]|!\[CDATA\[)/
const CONTINUATION = /^(?: {2,}|\t)/
/** The tags on a line with their quoted attribute values emptied, found by the policy's own bounded tag scan. */
function tagHeads(line: string): string {
  let heads = ''
  for (let at = line.indexOf('<'); at !== -1; at = line.indexOf('<', at + 1)) {
    const closed = tagEnd(line, at)
    // A tag whose `>` is on a later line (`<details\n>`) still opens here.
    const end = closed !== -1 ? closed : (line.indexOf('<', at + 1) + 1 || line.length + 1) - 2
    heads += line.slice(at, end + 1).replace(/"[^"]*"|'[^']*'/g, '""') + ' '
    at = end
  }
  return heads
}
const CONTAINERS = 'blockquote|dd|details|div|dl|dt|h[1-6]|li|ol|p|pre|summary|table|tbody|td|tfoot|th|thead|tr|ul'
/** HTML that holds blank lines inside it: containers, and the elements the policy drops with their content. */
const HOLDING = new RegExp(`<(/?)(${CONTAINERS}|${HTML_POLICY.dropWithContent.join('|')})(?=[\\s>/]|$)`, 'gi')

/**
 * Cuts a document at top-level block boundaries, so a settled block is parsed once however long the document grows.
 * Parsing the blocks one by one has to equal parsing the document: when a construct could reach across a blank line
 * (definitions, comments, raw HTML, open `<details>`, a list that continues), there is no cut. Skipping a cut only costs
 * time. `markdown-source.unit.test.mjs` proves the equivalence against a real parser.
 */
export function splitMarkdownBlocks(text: string): readonly string[] {
  if (DEFINITION.test(text)) return [text]
  const blocks: string[] = []
  let blockStart = 0
  let fence: { readonly char: string; readonly length: number; readonly listed: boolean } | undefined
  let rawUntil: string | undefined
  const holding = new Map<string, number>()
  let held = 0
  let blank = false
  let inList = false
  let lineStart = 0
  while (lineStart < text.length) {
    const newline = text.indexOf('\n', lineStart)
    const lineEnd = newline === -1 ? text.length : newline
    const line = text.slice(lineStart, lineEnd)
    const start = lineStart
    lineStart = lineEnd + 1
    if (fence !== undefined) {
      const trimmed = line.trim()
      // A fence inside a list item can end with the item. Which list offset it belongs to is not known here, so stop cutting.
      if (fence.listed && trimmed !== '' && !CONTINUATION.test(line)) break
      if (/^ {0,3}(?:`{3,}|~{3,})\s*$/.test(line) && trimmed[0] === fence.char && trimmed.length >= fence.length) fence = undefined
      continue
    }
    if (rawUntil !== undefined) {
      if (line.toLowerCase().includes(rawUntil)) rawUntil = undefined
      continue
    }
    if (line.trim() === '') { blank = true; continue }
    const isList = LIST_LINE.test(line)
    if (blank && held === 0 && !CONTINUATION.test(line) && !(inList && isList)) {
      blocks.push(text.slice(blockStart, start))
      blockStart = start
      inList = false
    }
    blank = false
    if (isList) inList = true
    const opening = FENCE_OPEN.exec(line)
    if (opening !== null && !(opening[2]![0] === '`' && opening[3]!.includes('`'))) {
      const listed = inList && opening[1]!.length >= 2
      if (!listed) inList = false
      fence = { char: opening[2]![0]!, length: opening[2]!.length, listed }
      continue
    }
    if (!line.includes('<')) continue
    const comment = line.lastIndexOf('<!--')
    if (comment !== -1 && !line.includes('-->', comment)) { rawUntil = '-->'; continue }
    const raw = RAW_HTML_OPEN.exec(line)?.[1]?.toLowerCase()
    if (raw !== undefined && !line.toLowerCase().includes(`</${raw}`)) { rawUntil = `</${raw}`; continue }
    if (RAW_DECLARATION.test(line) && !line.includes('>')) { rawUntil = '>'; continue }
    // An HTML container left open (a disclosure, a quote, a table) holds the blank lines inside it.
    // Per name, so a stray close of one element cannot end another; quoted attribute values are not tags.
    // Comments and quoted attribute values hold no tags.
    for (const [, slash, tag] of tagHeads(line.replace(/<!--[\s\S]*?-->/g, '')).matchAll(HOLDING)) {
      const name = tag!.toLowerCase()
      const open = holding.get(name) ?? 0
      if (slash === '') { holding.set(name, open + 1); held += 1 } else if (open > 0) { holding.set(name, open - 1); held -= 1 }
    }
  }
  blocks.push(text.slice(blockStart))
  return blocks
}

interface SourceNode { type: string; value?: string; children?: SourceNode[] }

/** Each line break in prose stays a line break, as in a GitHub comment, for text written as lines rather than paragraphs. */
export function remarkHardBreaks() {
  const visit = (parent: SourceNode): void => {
    if (parent.children === undefined) return
    parent.children = parent.children.flatMap((child): SourceNode[] => {
      if (child.type !== 'text' || !child.value?.includes('\n')) { visit(child); return [child] }
      return child.value.split('\n').flatMap((value, index): SourceNode[] => [...(index === 0 ? [] : [{ type: 'break' }]), ...(value === '' ? [] : [{ type: 'text', value }])])
    })
  }
  return (tree: SourceNode) => visit(tree)
}

/**
 * Inline Markdown as the words it renders to, for one-line labels that cannot carry formatting (rows, titles, accessible
 * names) and must not load the parser. Code keeps its text, emphasis and link markers go, an image keeps its alt text.
 */
export function markdownPlainText(markdown: string): string {
  const code: string[] = []
  const protect = (value: string): string => `\u0000${code.push(value) - 1}\u0001`
  const source = markdown.split('\n').map(line => line.trim()).join(' ')
    .replace(/^(?:#{1,6}\s+|>\s*|[-*+]\s+(?:\[[ xX]\]\s+)?|\d{1,9}[.)]\s+)/, '')
  const runs = [...source.matchAll(/`+/g)]
  const next = new Map<number, number>()
  const closing = new Map<number, number>()
  for (let index = runs.length - 1; index >= 0; index--) {
    const run = runs[index]!
    const end = next.get(run[0].length)
    if (end !== undefined) closing.set(run.index, end)
    next.set(run[0].length, run.index)
  }
  const pieces: string[] = []
  let at = 0
  for (const match of source.matchAll(/\\[\\`*_{}[\]()#+\-.!|<>~&]|`+/g)) {
    if (match.index < at) continue
    pieces.push(source.slice(at, match.index))
    const token = match[0]
    const end = token[0] === '`' ? closing.get(match.index) : undefined
    if (token[0] === '\\') { pieces.push(protect(token[1]!)); at = match.index + 2 }
    else if (end !== undefined) { pieces.push(protect(source.slice(match.index + token.length, end).trim())); at = end + token.length }
    else { pieces.push(token); at = match.index + token.length }
  }
  pieces.push(source.slice(at))
  let text = pieces.join('')
  // Destinations may contain '[' (IPv6, for example). Index the next ')' instead of retrying a destination scan at
  // every opening label when the document has no closing parenthesis.
  const closeParen = new Int32Array(text.length + 1)
  let end = -1
  for (let index = text.length; index >= 0; index--) {
    if (text[index] === ')') end = index
    closeParen[index] = end
  }
  const links: string[] = []
  at = 0
  for (const match of text.matchAll(/!?\[([^[\]]*)\]\(/g)) {
    if (match.index < at || (match[1] === '' && match[0][0] !== '!')) continue
    const close = closeParen[match.index + match[0].length]!
    if (close === -1) continue
    links.push(text.slice(at, match.index), match[1]!)
    at = close + 1
  }
  links.push(text.slice(at))
  text = stripPairs(links.join(''), /\*\*|__/g, false)
  text = stripPairs(text, /[*_]/g, true)
  text = stripPairs(text, /~~/g, false)
  return text.replace(/\u0000(\d+)\u0001/g, (_, index: string) => code[Number(index)] ?? '').trim()
}

/** Three bounded passes replace the old backtracking emphasis expressions; unmatched syntax remains literal. */
function stripPairs(text: string, pattern: RegExp, single: boolean): string {
  const pending = new Map<string, number>()
  const removed = new Uint8Array(text.length)
  for (const match of text.matchAll(pattern)) {
    const token = match[0]
    const index = match.index
    const before = text[index - 1] ?? ''
    const after = text[index + token.length] ?? ''
    const opening = pending.get(token)
    if (opening !== undefined && index > opening + token.length && /\S/.test(before) && (!single || !/[\w*]/.test(after))) {
      removed[opening] = token.length
      removed[index] = token.length
      pending.delete(token)
      continue
    }
    if (single) pending.clear()
    if (/\S/.test(after) && (!single || !/[\w*]/.test(before)) && (single || opening === undefined)) pending.set(token, index)
  }
  const pieces: string[] = []
  let at = 0
  for (let index = 0; index < removed.length; index++) {
    if (removed[index] === 0) continue
    pieces.push(text.slice(at, index))
    at = index + removed[index]!
  }
  pieces.push(text.slice(at))
  return pieces.join('')
}

const MAX_CONTAINER_MARKERS = 32
const MAX_INDENT = 128
const CONTAINER_MARKER = /[ \t]*(?:>|[-*+](?=[ \t])|\d{1,9}[.)](?=[ \t]))/y
/**
 * Bounds what the parser has to nest before it runs: past 32 container markers (quote `>` and list markers, in any
 * mix) or 128 columns of indentation a line keeps its words but loses the extra nesting. Thousands of nested quotes
 * otherwise overflow the parser's stack, and deeply indented lists take seconds. Fenced code at the top level keeps
 * its indentation.
 */
export function limitNesting(text: string): string {
  if (!/^[ \t>*+\-\d.)]{64}/m.test(text)) return text
  let fence: { readonly char: string; readonly length: number } | undefined
  return text.split('\n').map(line => {
    const marker = /^ {0,3}(`{3,}|~{3,})/.exec(line)?.[1]
    if (fence !== undefined) {
      if (marker !== undefined && marker[0] === fence.char && marker.length >= fence.length && line.trim() === marker) fence = undefined
      return line
    }
    if (marker !== undefined && !(marker[0] === '`' && line.slice(line.indexOf(marker) + marker.length).includes('`'))) { fence = { char: marker[0]!, length: marker.length }; return line }
    let end = 0
    let markers = 0
    let kept = 0
    CONTAINER_MARKER.lastIndex = 0
    for (let match = CONTAINER_MARKER.exec(line); match !== null; match = CONTAINER_MARKER.exec(line)) {
      markers += 1
      end = CONTAINER_MARKER.lastIndex
      if (markers === MAX_CONTAINER_MARKERS) kept = end
    }
    if (markers > MAX_CONTAINER_MARKERS) line = line.slice(0, kept) + ' ' + line.slice(end).trimStart()
    const indent = /^[ \t]*/.exec(line)![0]
    const columns = indent.replace(/\t/g, '    ').length
    return columns > MAX_INDENT ? ' '.repeat(MAX_INDENT) + line.slice(indent.length) : line
  }).join('\n')
}
