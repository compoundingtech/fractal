/**
 * pulldown-cmark, compiled to WebAssembly (`packages/fractal-ui/markdown-wasm`), as a drop-in parser for the kit's
 * Markdown. It replaces only the parse step: its events become the same syntax tree (mdast) remark builds, and the HTML
 * policy, hard breaks, streaming tail and React components run unchanged on it.
 *
 * The module loads lazily, after the first Markdown mounts, and never from the shell. Until it is ready, and for good if
 * it fails to load, Markdown keeps the JavaScript parser. Each Markdown picks its parser once, at mount, so a mounted
 * block never re-renders because the module arrived.
 */

interface MdastNode { type: string; value?: string; children?: MdastNode[]; [field: string]: unknown }
interface PulldownExports {
  readonly memory: WebAssembly.Memory
  readonly alloc: (len: number) => number
  readonly dealloc: (pointer: number, len: number) => void
  readonly parse: (pointer: number, len: number) => number
  readonly result_ptr: () => number
}
type Event = readonly [string, ...string[]]

let exports: PulldownExports | undefined
let loading: Promise<boolean> | undefined
let enabled = true

/** Whether new Markdown mounts parse with pulldown-cmark. */
export const pulldownReady = (): boolean => enabled && exports !== undefined

/** Instantiates the module from its bytes. Tests and hosts that already hold the bytes call this directly. */
export async function installPulldown(bytes: BufferSource): Promise<void> {
  const { instance } = await WebAssembly.instantiate(bytes, {})
  exports = instance.exports as unknown as PulldownExports
}

/** Loads the committed module once; resolves `false`, and keeps the JavaScript parser, if it cannot or pulldown is off. */
export function loadPulldown(): Promise<boolean> {
  // Off requests nothing, so a host without network access (the product embed) never fetches the module.
  if (!enabled) return Promise.resolve(false)
  loading ??= (async () => {
    try {
      const response = await fetch(new URL('./markdown-pulldown.generated.wasm', import.meta.url))
      if (!response.ok) return false
      await installPulldown(await response.arrayBuffer())
      return true
    } catch {
      return false
    }
  })()
  return loading
}

/** Turns pulldown off (or on again) for new mounts and loads, for a kill switch, a comparison or a host without network. */
export const setPulldownEnabled = (on: boolean): void => { enabled = on }

const encoder = new TextEncoder()
const decoder = new TextDecoder()

function events(source: string): readonly Event[] {
  const module = exports!
  const bytes = encoder.encode(source)
  const pointer = module.alloc(bytes.length)
  try {
    new Uint8Array(module.memory.buffer, pointer, bytes.length).set(bytes)
    const length = module.parse(pointer, bytes.length)
    return JSON.parse(decoder.decode(new Uint8Array(module.memory.buffer, module.result_ptr(), length))) as Event[]
  } finally {
    module.dealloc(pointer, bytes.length)
  }
}

const PHRASING = new Set(['text', 'emphasis', 'strong', 'delete', 'inlineCode', 'link', 'image', 'break', 'html', 'footnoteReference'])

/** The mdast tree remark would build for `source`, from pulldown-cmark's events. */
export function pulldownTree(source: string): MdastNode {
  const root: MdastNode = { type: 'root', children: [] }
  const stack: MdastNode[] = [root]
  const top = () => stack.at(-1)!
  const append = (node: MdastNode) => {
    let parent = top()
    // pulldown leaves out the paragraph of a tight list item; mdast always has one.
    if (parent.type === 'listItem' && PHRASING.has(node.type)) {
      const last = parent.children!.at(-1)
      if (last?.type === 'paragraph' && last['implicit'] === true) parent = last
      else { const paragraph: MdastNode = { type: 'paragraph', children: [], implicit: true }; parent.children!.push(paragraph); parent = paragraph }
    }
    const previous = parent.children!.at(-1)
    if (node.type === 'text' && previous?.type === 'text') { previous.value += node.value!; return }
    parent.children!.push(node)
  }
  // Inside an image everything is its alt text: nested emphasis, code and breaks add words, not nodes.
  let inImage = 0
  for (const [kind, ...fields] of literalEvents(source)) {
    if (inImage > 0) {
      if (kind === 's') inImage += 1
      else if (kind === 'e') { inImage -= 1; if (inImage === 0) stack.pop() }
      else if (kind === 't' || kind === 'c') top().alt += fields[0]!
      else if (kind === 'sb' || kind === 'hb') top().alt += ' '
      continue
    }
    switch (kind) {
      case 's': {
        const [type, first, second] = fields as [string, string | undefined, string | undefined]
        const node: MdastNode =
          type === 'heading' ? { type, depth: Number(first), children: [] }
          : type === 'code' ? { type, lang: first?.trim().split(/\s+/)[0] || null, meta: first?.trim().split(/\s+/).slice(1).join(' ') || null, value: '' }
          : type === 'html' ? { type, value: '' }
          : type === 'list' ? { type, ordered: first !== undefined, start: first === undefined ? null : Number(first), spread: false, children: [] }
          : type === 'listItem' ? { type, spread: false, checked: null, children: [] }
          : type === 'footnoteDefinition' ? { type, identifier: first!.toLowerCase(), label: first, children: [] }
          : type === 'table' ? { type, align: fields.slice(1).map(align => align === '' ? null : align), children: [] }
          : type === 'tableHead' ? { type: 'tableRow', children: [] }
          : type === 'link' ? { type, url: first ?? '', title: second || null, children: [] }
          : type === 'image' ? { type, url: first ?? '', title: second || null, alt: '' }
          : { type, children: [] }
        if (type === 'paragraph' && top().type === 'listItem') top().spread = true
        if (type !== 'span') append(node)
        stack.push(node)
        if (type === 'image') inImage = 1
        break
      }
      case 'e': {
        const node = stack.pop()!
        if (node.type === 'code' || node.type === 'html') node.value = node.value!.replace(/\n$/, '')
        if (node.type === 'list') node.spread = node.children!.some(item => item.spread === true)
        if (node.type === 'span') for (const child of node.children ?? []) append(child)
        break
      }
      case 't': {
        const node = top()
        if (node.type === 'code' || node.type === 'html') node.value += fields[0]!
        else if (node.type === 'image') node.alt += fields[0]!
        else append({ type: 'text', value: fields[0]! })
        break
      }
      case 'c': if (top().type === 'image') top().alt += fields[0]!; else append({ type: 'inlineCode', value: fields[0]! }); break
      case 'h': if (top().type === 'html') top().value += fields[0]!; else append({ type: 'html', value: fields[0]! }); break
      case 'sb': if (top().type === 'image') top().alt += ' '; else append({ type: 'text', value: '\n' }); break
      case 'hb': append({ type: 'break' }); break
      case 'r': append({ type: 'thematicBreak' }); break
      case 'fr': append({ type: 'footnoteReference', identifier: fields[0]!.toLowerCase(), label: fields[0]! }); break
      case 'tk': { const item = stack.findLast(node => node.type === 'listItem'); if (item !== undefined) item.checked = fields[0] === 'x'; break }
    }
  }
  linkLiterals(root)
  return root
}

interface LiteralUrl { readonly index: number; readonly value: string }

/**
 * GFM's protocol/www boundaries, without restarting the trailing-punctuation scan at every punctuation character.
 * Each candidate is scanned forward once and its trail backward once. Email is deliberately a separate scan.
 */
function* literalUrls(source: string): Generator<LiteralUrl> {
  const prefix = /https?:\/\/|www\./gi
  let tokenEnd = 0
  let trailStart = 0
  let trail = new Uint8Array(0)
  let domainEnd = 0
  let domainAt = 0
  let lastUnderscore = -1
  for (let match = prefix.exec(source); match !== null; match = prefix.exec(source)) {
    const previous = source[match.index - 1] ?? ''
    const www = /^www/i.test(match[0])
    if (www ? previous !== '' && !/[\t\n\r (*_[\]~]/.test(previous) : /[a-z]/i.test(previous)) continue
    const domainStart = match.index + (www ? 0 : match[0].length)
    if (!www && /[\s\p{P}\p{S}\p{Cc}]/u.test(String.fromCodePoint(source.codePointAt(domainStart) ?? 0))) continue
    const freshDomain = domainStart >= domainEnd
    if (freshDomain) {
      domainEnd = domainStart
      while (domainEnd < source.length && /[-._]|[^\s\p{P}\p{S}]/u.test(String.fromCodePoint(source.codePointAt(domainEnd)!))) {
        domainEnd += source.codePointAt(domainEnd)! > 0xffff ? 2 : 1
      }
    }
    if (match.index >= tokenEnd) {
      trailStart = match.index
      tokenEnd = domainEnd
      while (tokenEnd < source.length && !/[\s<]/u.test(source[tokenEnd]!)) tokenEnd++
      // Dynamic suffix recognition is equivalent to GFM's speculative `trail` tokenizer, but never rescans a suffix.
      // Cache the whole whitespace-delimited run, including when a `](` ends one link before another starts.
      trail = new Uint8Array(tokenEnd - trailStart + 1)
      trail[tokenEnd - trailStart] = 1
      for (let index = tokenEnd - 1; index >= trailStart; index--) {
        const character = source[index]!
        if (/[!"')*,.:;?_~]/.test(character)) trail[index - trailStart] = trail[index + 1 - trailStart]!
        else if (character === ']') trail[index - trailStart] = /[([]/.test(source[index + 1] ?? '') ? 1 : trail[index + 1 - trailStart]!
        else if (character === '&') {
          let after = index + 1
          while (after < tokenEnd && /[a-z]/i.test(source[after]!)) after++
          if (after > index + 1 && source[after] === ';') trail[index - trailStart] = trail[after + 1 - trailStart]!
        }
      }
    }
    if (freshDomain) {
      domainAt = domainStart
      while (domainAt < domainEnd) {
        if ((source[domainAt] === '.' || source[domainAt] === '_') && trail[domainAt - trailStart]) break
        domainAt++
      }
      // Only underscores in the last two domain segments invalidate a URL. Cache their last position so a failed
      // `www._www._…` candidate cannot rescan the same domain at every later prefix.
      lastUnderscore = -1
      let dots = 0
      for (let index = domainAt - 1; index >= domainStart && dots < 2; index--) {
        if (source[index] === '.') dots++
        else if (source[index] === '_' && lastUnderscore < 0) lastUnderscore = index
      }
    }
    if (domainAt <= domainStart || lastUnderscore >= domainStart || (www && match.index + match[0].length === source.length)) continue
    let at = domainAt
    let opens = 0
    let closes = 0
    while (at < tokenEnd) {
      const character = source[at]!
      if (character === '(') opens++
      else if (character === ')' && closes < opens) closes++
      else {
        if (trail[at - trailStart]) break
        if (character === ')') closes++
      }
      at++
    }
    yield { index: match.index, value: source.slice(match.index, at) }
    prefix.lastIndex = at
  }
}

/**
 * The first event pass supplies source offsets and opaque contexts, not the already-parsed URL text. Turn eligible
 * raw URL extents into escaped explicit links and parse once more: emphasis/code/delete can no longer consume their
 * delimiters, and surrounding emphasis is parsed with the same precedence as GFM. Code, HTML, existing links and
 * image alt text are never rewritten. Documents without literal URLs need only the original pass.
 */
function literalEvents(source: string): readonly Event[] {
  const candidates = [...literalUrls(source)]
  const original = events(source)
  if (candidates.length === 0) return original
  const ranges: { start: number; end: number }[] = []
  const stack: string[] = []
  let opaque = 0
  for (const [kind, first, start, end] of original) {
    if (kind === 's') { stack.push(first!); if (['code', 'html', 'link', 'image'].includes(first!)) opaque++ }
    else if (kind === 'e') { if (['code', 'html', 'link', 'image'].includes(stack.pop()!)) opaque-- }
    else if (kind === 't' && opaque === 0 && start !== undefined) ranges.push({ start: Number(start), end: Number(end) })
  }
  const escape = (value: string) => value.replace(/[!-/:-@[-`{-~]/g, '\\$&')
  const parts: string[] = []
  let byte = 0
  let cursor = 0
  let range = 0
  let at = 0
  for (const candidate of candidates) {
    byte += encoder.encode(source.slice(cursor, candidate.index)).length
    cursor = candidate.index
    while (range < ranges.length && ranges[range]!.end <= byte) range++
    if (range === ranges.length || ranges[range]!.start > byte) continue
    let before = source.slice(at, candidate.index)
    // Inserting '[' must not turn a literal preceding '!' into an image, or let a literal '\' escape the link.
    const marker = source[candidate.index - 1]
    let slash = candidate.index - (marker === '!' ? 1 : 0)
    while (slash > at && source[slash - 1] === '\\') slash--
    const slashCount = candidate.index - (marker === '!' ? 1 : 0) - slash
    if (marker === '!' && slashCount % 2 === 0) before = `${before.slice(0, -1)}\\!`
    else if (marker === '\\' && slashCount % 2 !== 0) before += '\\'
    parts.push(before, `[${escape(candidate.value)}](<${escape(/^www/i.test(candidate.value) ? `http://${candidate.value}` : candidate.value)}>)`)
    at = candidate.index + candidate.value.length
  }
  if (parts.length === 0) return original
  parts.push(source.slice(at))
  return events(parts.join(''))
}

/** Email is the only literal syntax absent from both parsers; protocol/www links are recognized before emphasis. */
function linkLiterals(parent: MdastNode): void {
  if (parent.children === undefined || parent.type === 'link' || parent.type === 'linkReference') return
  parent.children = parent.children.flatMap((child): MdastNode[] => {
    if (child.type !== 'text') { linkLiterals(child); return [child] }
    return emailLiterals(child)
  })
}

/** A maximal email-shaped token is validated once, never retried at each dot when no '@' follows. */
function emailLiterals(node: MdastNode): MdastNode[] {
  const source = node.value!
  const parts: MdastNode[] = []
  let at = 0
  for (const match of source.matchAll(/[-.\w+@]+/g)) {
    let end = match[0].length
    while (match[0][end - 1] === '.') end--
    const candidate = match[0].slice(0, end)
    if (!/^[-.\w+]+@[-\w]+(?:\.[-\w]+)+$/.test(candidate) || !/[A-Za-z]$/.test(candidate) || /[\w/]/.test(source[match.index - 1] ?? '')) continue
    if (match.index > at) parts.push({ type: 'text', value: source.slice(at, match.index) })
    parts.push({ type: 'link', url: `mailto:${candidate}`, title: null, children: [{ type: 'text', value: candidate }] })
    at = match.index + candidate.length
  }
  if (parts.length === 0) return [node]
  if (at < source.length) parts.push({ type: 'text', value: source.slice(at) })
  return parts
}

/**
 * Replace only GFM's restarting email tokenizer and postparse regex with a bounded tree scan. Retain GFM's protocol
 * and www tokenizers: their precedence over inline delimiters is part of the syntax, not a postparse link policy.
 */
export function remarkLinearAutolinks(this: { data(): object }) {
  const data = this.data()
  const extensions = 'micromarkExtensions' in data ? data.micromarkExtensions : undefined
  const disabled = { disable: { null: ['emailAutolink'] } }
  if (Array.isArray(extensions)) extensions.push(disabled)
  else Object.assign(data, { micromarkExtensions: [disabled] })
  const replaceTransform = (extension: unknown): void => {
    if (Array.isArray(extension)) { for (const item of extension) replaceTransform(item); return }
    if (typeof extension !== 'object' || extension === null || !('enter' in extension)) return
    const enter = extension.enter
    if (typeof enter === 'object' && enter !== null && 'literalAutolink' in enter && 'transforms' in extension) extension.transforms = []
  }
  replaceTransform('fromMarkdownExtensions' in data ? data.fromMarkdownExtensions : undefined)
  return (tree: MdastNode) => linkLiterals(tree)
}

/** A remark step that makes pulldown-cmark the parser of the processor it joins. */
export function remarkPulldown(this: { parser?: (document: string) => MdastNode }) {
  this.parser = (document: string) => pulldownTree(String(document))
}
