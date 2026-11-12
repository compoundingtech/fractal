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
  for (const [kind, ...fields] of events(source)) {
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

const LITERAL = /\b(?:https?:\/\/|www\.)[^\s<]+/g
/** GFM's autolink literals (`https://…`, `www.…`), which pulldown-cmark leaves as text. */
function linkLiterals(parent: MdastNode): void {
  if (parent.children === undefined || parent.type === 'link' || parent.type === 'linkReference') return
  parent.children = parent.children.flatMap((child): MdastNode[] => {
    if (child.type !== 'text') { linkLiterals(child); return [child] }
    const parts: MdastNode[] = []
    let at = 0
    for (const match of child.value!.matchAll(LITERAL)) {
      const candidate = match[0]
      let balance = 0
      for (const character of candidate) { if (character === '(') balance++; else if (character === ')') balance-- }
      let end = candidate.length
      while (end > 0) {
        const last = candidate[end - 1]!
        if (last === ')' && balance < 0) { balance++; end--; continue }
        if (/[?!.,:;*_~'"\]]/.test(last)) { end--; continue }
        break
      }
      const url = candidate.slice(0, end)
      if (url.length <= (candidate.startsWith('www.') ? 4 : candidate.startsWith('https://') ? 8 : 7)) continue
      if (match.index > at) parts.push({ type: 'text', value: child.value!.slice(at, match.index) })
      parts.push({ type: 'link', url: url.startsWith('www.') ? `http://${url}` : url, title: null, children: [{ type: 'text', value: url }] })
      at = match.index + url.length
    }
    if (parts.length === 0) return [child]
    if (at < child.value!.length) parts.push({ type: 'text', value: child.value!.slice(at) })
    return parts
  })
}

/** A remark step that makes pulldown-cmark the parser of the processor it joins. */
export function remarkPulldown(this: { parser?: (document: string) => MdastNode }) {
  this.parser = (document: string) => pulldownTree(String(document))
}
