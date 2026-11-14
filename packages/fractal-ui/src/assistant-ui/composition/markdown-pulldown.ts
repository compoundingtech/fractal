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

function events(source: string | Uint8Array): readonly Event[] {
  const module = exports!
  const bytes = typeof source === 'string' ? encoder.encode(source) : source
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
  // CommonMark §2.3: normalize before either parser pass so all UTF-8 source offsets
  // refer to the same replacement bytes, including literal-URL source rewrites.
  source = source.replace(/\0/g, '\uFFFD')
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
      case 'h': {
        if (top().type === 'html') top().value += fields[0]!
        else {
          const value = fields[0]!
          // pulldown accepts FF in inline closing tags; CommonMark/remark permits only space, TAB and line endings.
          // Keep invalid closers as text so the policy cannot end a hidden region earlier than remark does.
          const type = value.startsWith('</') && !/^<\/[A-Za-z][A-Za-z0-9-]*[\t\n\r ]*>$/.test(value) ? 'text' : 'html'
          append({ type, value })
        }
        break
      }
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
 * Text events identify both the original text span and its enclosing inline run. Recognize URL extents only within
 * that run, never across structural cell delimiters, then rewrite eligible raw extents as escaped explicit links.
 * Reparse so emphasis/code/delete cannot consume URL delimiters. Existing links, images, code and HTML stay opaque.
 * Preserve the raw literal, including escaped pipes: GFM keeps their backslash inside URL destinations and labels.
 */
function literalEvents(source: string): readonly Event[] {
  const bytes = encoder.encode(source)
  const original = events(bytes)
  if (!/https?:\/\/|www\./i.test(source)) return original
  interface Rewrite { readonly start: number; readonly end: number; readonly value: string }
  interface InlineRun {
    readonly start: number; readonly end: number
    readonly candidates: Rewrite[]; next: number
  }
  const runs = new Map<string, InlineRun>()
  const ranges: { start: number; end: number; run: InlineRun }[] = []
  const stack: string[] = []
  let opaque = 0
  for (const [kind, first, start, end, inlineStart, inlineEnd] of original) {
    if (kind === 's') { stack.push(first!); if (['code', 'html', 'link', 'image'].includes(first!)) opaque++ }
    else if (kind === 'e') { if (['code', 'html', 'link', 'image'].includes(stack.pop()!)) opaque-- }
    else if (kind === 't' && opaque === 0 && inlineStart !== undefined && inlineEnd !== undefined) {
      const key = `${inlineStart}:${inlineEnd}`
      let run = runs.get(key)
      if (run === undefined) {
        run = { start: Number(inlineStart), end: Number(inlineEnd), candidates: [], next: 0 }
        runs.set(key, run)
      }
      ranges.push({ start: Number(start), end: Number(end), run })
    }
  }
  for (const run of runs.values()) {
    const raw = decoder.decode(bytes.subarray(run.start, run.end))
    let cursor = 0
    let byte = run.start
    for (const candidate of literalUrls(raw)) {
      const start = candidate.index
      const end = start + candidate.value.length
      byte += encoder.encode(raw.slice(cursor, start)).length
      const byteEnd = byte + encoder.encode(raw.slice(start, end)).length
      run.candidates.push({ start: byte, end: byteEnd, value: candidate.value })
      cursor = end
      byte = byteEnd
    }
  }
  const escape = (value: string, destination = false) => value.replace(/[!-/:-@[-`{-~]/g, (character, index: number) => {
    if (character !== '|') return `\\${character}`
    // The destination must not duplicate the label's structural pipes. Character references decode to the same URL.
    if (destination) return '&#124;'
    // Preserve block parsing too: escaping an originally structural pipe in a paragraph could turn a malformed
    // table header into a valid one on the second pass. In cells, retain the original odd escaped-pipe parity.
    let slash = index
    while (slash > 0 && value[slash - 1] === '\\') slash--
    return (index - slash) % 2 === 0 ? '|' : '\\|'
  })
  const parts: string[] = []
  let at = 0
  // Text ranges arrive in source order. Consume each run's candidate cursor here rather than sorting overlapping
  // item/paragraph runs, so recognition and reconstruction remain bounded scans.
  for (const range of ranges) {
    const run = range.run
    while (run.next < run.candidates.length && run.candidates[run.next]!.start < range.start) run.next++
    while (run.next < run.candidates.length && run.candidates[run.next]!.start < range.end) {
      const candidate = run.candidates[run.next++]!
      if (candidate.start < at) continue
      let before = decoder.decode(bytes.subarray(at, candidate.start))
      // Inserting '[' must not turn a literal preceding '!' into an image, or let a literal '\' escape the link.
      const marker = before.at(-1)
      let slash = before.length - (marker === '!' ? 1 : 0)
      while (slash > 0 && before[slash - 1] === '\\') slash--
      const slashCount = before.length - (marker === '!' ? 1 : 0) - slash
      if (marker === '!' && slashCount % 2 === 0) before = `${before.slice(0, -1)}\\!`
      else if (marker === '\\' && slashCount % 2 !== 0) before += '\\'
      parts.push(before, `[${escape(candidate.value)}](<${escape(/^www/i.test(candidate.value) ? `http://${candidate.value}` : candidate.value, true)}>)`)
      at = candidate.end
    }
  }
  if (parts.length === 0) return original
  parts.push(decoder.decode(bytes.subarray(at)))
  return events(parts.join(''))
}

/**
 * GFM also has a URL-only postparse fallback (for example `|www.…` in a paragraph). Preserve its distinct preceding
 * character, ASCII-domain and punctuation rules, without its restarting regex or the quadratic email alternatives.
 */
function* fallbackUrls(source: string): Generator<LiteralUrl> {
  const prefix = /https?:\/\/|www(?=\.)/gi
  const domainPattern = /[\w-]*\.[\w.-]*/y
  for (let match = prefix.exec(source); match !== null; match = prefix.exec(source)) {
    const previous = source[match.index - 1]
    if (previous !== undefined && (/[0-9a-z]/i.test(previous) || !/[\s\p{P}\p{S}]/u.test(previous))) continue
    domainPattern.lastIndex = match.index + match[0].length
    const domainMatch = domainPattern.exec(source)
    if (domainMatch === null) continue
    let end = domainPattern.lastIndex
    while (end < source.length && !/[\t\n\r ]/.test(source[end]!)) end++
    // A rejected replacement still consumes the original regex's whole match; never retry each prefix in its path.
    prefix.lastIndex = end
    const www = /^www/i.test(match[0])
    const domainStart = www ? match.index : match.index + match[0].length
    const domain = source.slice(domainStart, domainPattern.lastIndex)
    if (/[\p{P}\p{S}]/u.test(domain[0]!) || domain.split('.').slice(-2).some(label => label !== '' && (label.includes('_') || !/[0-9a-z]/i.test(label)))) continue
    const pathEnd = end
    while (end > domainStart && /[!"&'),.:;<>?\]}]/.test(source[end - 1]!)) end--
    let balance = 0
    for (let index = domainStart; index < end; index++) {
      if (source[index] === '(') balance++
      else if (source[index] === ')') balance--
    }
    for (let index = end; index < pathEnd && balance > 0; index++) {
      if (source[index] === ')') { balance--; end = index + 1 }
    }
    yield { index: match.index, value: source.slice(match.index, end) }
  }
}

/** Tokenizer links are already nodes. Only the URL fallback and bounded email scan operate on remaining text. */
function linkLiterals(parent: MdastNode): void {
  if (parent.children === undefined || parent.type === 'link' || parent.type === 'linkReference') return
  parent.children = parent.children.flatMap((child): MdastNode[] => {
    if (child.type !== 'text') { linkLiterals(child); return [child] }
    const parts: MdastNode[] = []
    let at = 0
    for (const candidate of fallbackUrls(child.value!)) {
      if (candidate.index > at) parts.push(...emailLiterals({ type: 'text', value: child.value!.slice(at, candidate.index) }))
      parts.push({ type: 'link', url: /^www/i.test(candidate.value) ? `http://${candidate.value}` : candidate.value, title: null, children: [{ type: 'text', value: candidate.value }] })
      at = candidate.index + candidate.value.length
    }
    if (at === 0) return emailLiterals(child)
    if (at < child.value!.length) parts.push(...emailLiterals({ type: 'text', value: child.value!.slice(at) }))
    return parts
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
 * Replace GFM's restarting email tokenizer and combined postparse regex with bounded scans. Retain its protocol/www
 * tokenizers and the URL-only postparse fallback; only email's quadratic path changes implementation.
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
