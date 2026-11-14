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
  let details = 0
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
    if (blank && details === 0 && !CONTINUATION.test(line) && !(inList && isList)) {
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
    details = Math.max(0, details + (line.match(/<details[\s>]/gi)?.length ?? 0) - (line.match(/<\/details\s*>/gi)?.length ?? 0))
  }
  blocks.push(text.slice(blockStart))
  return blocks
}

interface SourceNode { type: string; value?: string; children?: SourceNode[] }

const HTML_ELEMENTS = new Set('a abbr address area article aside audio b base bdi bdo blockquote body br button canvas caption center cite code col colgroup data datalist dd del details dfn dialog dir div dl dt em embed fieldset figcaption figure font footer form h1 h2 h3 h4 h5 h6 head header hr html i iframe img input ins kbd label legend li link main map mark marquee menu meta meter nav noscript object ol optgroup option output p param picture pre progress q rp rt ruby s samp script search section select slot small source span strike strong style sub summary sup svg table tbody td template textarea tfoot th thead time title tr track tt u ul var video wbr'.split(' '))
const FLOW_PARENTS = new Set(['root', 'blockquote', 'listItem', 'footnoteDefinition'])

/** What a run of raw HTML says as text: tags of real elements and comments go, and `Array<string>` stays what it was written as. */
const visibleHtmlText = (html: string): string => html
  .replace(/<!--[\s\S]*?(?:-->|$)/g, '')
  .replace(/<(script|style)\b[\s\S]*?(?:<\/\1\s*>|$)/gi, '')
  .replace(/<\/?([A-Za-z][A-Za-z0-9-]*)(?:\s[^<>]*)?\/?>/g, (tag: string, name: string) => HTML_ELEMENTS.has(name.toLowerCase()) ? '' : tag)
  .trim()

/** Text never disappears for looking like a tag. No HTML reaches the page: the words go through as escaped text. */
export function remarkVisibleHtml() {
  const visit = (parent: SourceNode): void => {
    if (parent.children === undefined) return
    parent.children = parent.children.flatMap((child): SourceNode[] => {
      if (child.type !== 'html') { visit(child); return [child] }
      const value = visibleHtmlText(child.value ?? '')
      if (value === '') return []
      const text = { type: 'text', value }
      return FLOW_PARENTS.has(parent.type) ? [{ type: 'paragraph', children: [text] }] : [text]
    })
  }
  return (tree: SourceNode) => visit(tree)
}

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
