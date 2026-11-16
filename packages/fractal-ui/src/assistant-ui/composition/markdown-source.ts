import { HTML_POLICY } from './markdown-html-policy'
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
    for (const [, slash, tag] of line.replace(/<!--[\s\S]*?-->/g, '').replace(/<[^<>]*>/g, inner => inner.replace(/"[^"]*"|'[^']*'/g, '""')).matchAll(HOLDING)) {
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
  return markdown
    .replace(/\s*\n\s*/g, ' ')
    .replace(/^\s{0,3}(?:#{1,6}\s+|>\s*|[-*+]\s+(?:\[[ xX]\]\s+)?|\d{1,9}[.)]\s+)/, '')
    .replace(/(`+)([^`]|[^`][\s\S]*?[^`])\1(?!`)/g, (_, _fence: string, text: string) => `\u0000${code.push(text.trim()) - 1}\u0001`)
    .replace(/\\([\\`*_{}[\]()#+\-.!|<>~])/g, (_, character: string) => `\u0000${code.push(character) - 1}\u0001`)
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/(\*\*|__)(?=\S)([\s\S]*?\S)\1/g, '$2')
    .replace(/(^|[^\w*])([*_])(?=\S)([^*_]*?\S)\2(?![\w*])/g, '$1$3')
    .replace(/~~(?=\S)([\s\S]*?\S)~~/g, '$1')
    .replace(/\u0000(\d+)\u0001/g, (_, index: string) => code[Number(index)] ?? '')
    .trim()
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
