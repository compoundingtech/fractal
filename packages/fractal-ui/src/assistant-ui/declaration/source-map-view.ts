/**
 * Pure view model for one subject's source map: the KDL node paths of canonical KDL, TS↔KDL
 * links, frame strips, line diffs and the Why line. Host-agnostic: inputs are the structural
 * shape of Fractal's `source.mapped` claim (`SubjectSourceMap.map`), never a client DTO import.
 */

export interface SourcePosition { readonly line: number; readonly column: number }
export interface SourceSpan { readonly file: number; readonly start: SourcePosition; readonly end: SourcePosition }
/** A wire enum: a known word, or a newer emitter's word the client decoded as `Unknown`. */
export type OpenWord = string | { readonly _tag: 'Unknown'; readonly raw: string }
export interface SourceFrame extends SourceSpan { readonly role: OpenWord; readonly precision: OpenWord }
export interface SourceNode { readonly kdl: string; readonly span: SourceSpan; readonly frames: readonly SourceFrame[] }
export interface SourceFile { readonly repository: string; readonly path: string; readonly blob_hash: string }
export interface SourceMap {
  readonly subject: string
  readonly revision: string
  readonly repository: string
  readonly ref: string
  readonly sha: string
  readonly sequence: number
  readonly files: readonly SourceFile[]
  readonly declaration: number
  readonly inputs: readonly number[]
  readonly nodes: readonly SourceNode[]
}
export interface SourceRevision {
  readonly revision: string
  readonly sha: string
  readonly sequence: number
  readonly recorded_at_unix_ms: number
}

// ---------- KDL node paths ----------

/** Nodes keyed by their first argument; every other node is keyed by its index among same-named siblings. */
const NAMED: Record<string, true> = { agent: true, mission: true, step: true, gate: true }
const named = (name: string): boolean => Object.hasOwn(NAMED, name)

export interface KdlNodeLines { readonly path: string; readonly name: string; readonly label: string; readonly start: number; readonly end: number; readonly depth: number }
export interface KdlIndex {
  readonly nodes: readonly KdlNodeLines[]
  readonly byPath: ReadonlyMap<string, KdlNodeLines>
  /** Per line (index = line − 1): the outermost node starting on it, else the innermost node containing it. */
  readonly lineNode: readonly (string | undefined)[]
}

interface OpenNode { path: string; name: string; label: string; start: number; end: number; depth: number; counts: Map<string, number> }

/**
 * The node paths of KDL text, as the source map's `kdl` keys name them. A tolerant line-level
 * scanner for display linkage: it never validates, and an unparsable tail simply maps nothing.
 */
export const kdlIndex = (text: string): KdlIndex => {
  const nodes: OpenNode[] = []
  const root: OpenNode = { path: '', name: '', label: '', start: 0, end: 0, depth: -1, counts: new Map() }
  const stack: OpenNode[] = [root]
  let line = 1
  let current: { name: string; start: number; arg: string | undefined; argDone: boolean; started: OpenNode | undefined } | undefined
  let index = 0
  const finish = () => {
    if (current === undefined) return
    const node = open()
    node.end = line
    current = undefined
  }
  const open = (): OpenNode => {
    if (current!.started !== undefined) return current!.started
    const parent = stack.at(-1)!
    const same = parent.counts.get(current!.name) ?? 0
    parent.counts.set(current!.name, same + 1)
    const key = named(current!.name) && current!.arg !== undefined ? `${current!.name}[${current!.arg}]` : `${current!.name}[${same}]`
    const node: OpenNode = {
      path: parent.path === '' ? key : `${parent.path}/${key}`,
      name: current!.name,
      label: named(current!.name) && current!.arg !== undefined ? `${current!.name} ${current!.arg}` : current!.name,
      start: current!.start, end: line, depth: parent.depth + 1, counts: new Map(),
    }
    nodes.push(node)
    current!.started = node
    return node
  }
  const readString = (): string => {
    // index at the opening quote; returns the decoded-enough value (escapes kept verbatim but unquoted)
    let value = ''
    index += 1
    while (index < text.length && text[index] !== '"') {
      if (text[index] === '\\') { value += text.slice(index, index + 2); index += 2; continue }
      if (text[index] === '\n') line += 1
      value += text[index]
      index += 1
    }
    index += 1
    return value
  }
  const readRaw = (): string => {
    // r#"…"# or #"…"#: count hashes, read to the matching close.
    if (text[index] === 'r') index += 1
    let hashes = 0
    while (text[index] === '#') { hashes += 1; index += 1 }
    const close = `"${'#'.repeat(hashes)}`
    index += 1
    const end = text.indexOf(close, index)
    const stop = end === -1 ? text.length : end
    const value = text.slice(index, stop)
    for (const char of value) if (char === '\n') line += 1
    index = stop + close.length
    return value
  }
  const readBare = (): string => {
    const start = index
    while (index < text.length && !/[\s{};="]/.test(text[index]!)) index += 1
    return text.slice(start, index)
  }
  while (index < text.length) {
    const char = text[index]!
    if (char === '\n') { finish(); line += 1; index += 1; continue }
    if (char === ' ' || char === '\t' || char === '\r') { index += 1; continue }
    if (char === '\\' && text[index + 1] === '\n') { line += 1; index += 2; continue }
    if (char === '/' && text[index + 1] === '/') { while (index < text.length && text[index] !== '\n') index += 1; continue }
    if (char === '/' && text[index + 1] === '*') {
      const end = text.indexOf('*/', index + 2)
      const stop = end === -1 ? text.length : end + 2
      for (const c of text.slice(index, stop)) if (c === '\n') line += 1
      index = stop
      continue
    }
    if (char === '/' && text[index + 1] === '-') { index += 2; continue }
    if (char === ';') { finish(); index += 1; continue }
    if (char === '{') {
      if (current !== undefined) { stack.push(open()); current = undefined }
      index += 1
      continue
    }
    if (char === '}') {
      finish()
      if (stack.length > 1) stack.pop()!.end = line
      index += 1
      continue
    }
    const token = char === '"' ? readString()
      : (char === '#' || (char === 'r' && (text[index + 1] === '#' || text[index + 1] === '"'))) ? readRaw()
        : readBare()
    if (token === '' && char !== '"') { index += 1; continue }
    if (current === undefined) { current = { name: token, start: line, arg: undefined, argDone: false, started: undefined }; continue }
    if (text[index] === '=') {
      // A property: skip its value; it is never the identifying argument.
      index += 1
      if (text[index] === '"') readString()
      else if (text[index] === '#' || text[index] === 'r') readRaw()
      else readBare()
      continue
    }
    if (!current.argDone) { current.arg = token; current.argDone = true }
  }
  finish()
  const lines = text.endsWith('\n') ? text.split('\n').length - 1 : text.split('\n').length
  const containing: (string | undefined)[] = Array.from({ length: lines })
  const starting: (string | undefined)[] = Array.from({ length: lines })
  for (const node of nodes) {
    for (let at = node.start; at <= Math.min(node.end, lines); at += 1) containing[at - 1] = node.path
    starting[node.start - 1] ??= node.path
  }
  const done = nodes.map(({ counts: _counts, ...node }) => node)
  return { nodes: done, byPath: new Map(done.map(node => [node.path, node])), lineNode: containing.map((path, at) => starting[at] ?? path) }
}

/**
 * `[name, key]` per segment. Keys may contain `/` (`mission[land/code]`), so a key ends at the `]`
 * that ends the path or precedes the next `/`; mirrors `fractal_schema::source_map::locate::segments`.
 */
export const nodeSegments = (path: string): readonly (readonly [string, string])[] => {
  const segments: (readonly [string, string])[] = []
  let rest = path
  while (rest !== '') {
    const open = rest.indexOf('[')
    if (open === -1) { segments.push([rest, '']); break }
    const after = rest.slice(open + 1)
    let close = after.indexOf(']')
    while (close !== -1 && close + 1 < after.length && after[close + 1] !== '/') close = after.indexOf(']', close + 1)
    if (close === -1) close = after.length
    segments.push([rest.slice(0, open), after.slice(0, close)])
    rest = after.slice(close + 2)
  }
  return segments
}

const joinSegments = (segments: readonly (readonly [string, string])[]): string => segments.map(([name, key]) => `${name}[${key}]`).join('/')

/** The node itself when the map names it, else its nearest mapped ancestor: nodes without their own site inherit it. */
export const mappedAncestor = (path: string, mapped: ReadonlyMap<string, SourceNode>): string | undefined => {
  const segments = nodeSegments(path)
  for (let length = segments.length; length > 0; length -= 1) {
    const at = joinSegments(segments.slice(0, length))
    if (mapped.has(at)) return at
  }
  return undefined
}

/** `mission[x]/step[review]/goal[0]` → `step review › goal`. */
export const nodeLabel = (path: string): string => {
  const segments = nodeSegments(path)
  return segments.slice(1).map(([name, key]) => named(name) ? `${name} ${key}` : name).join(' › ') || (segments[0]?.[0] ?? path)
}

/** The node path of step run path `outer/nested-mission/inner` in mission `id`: nested steps alternate step and mission segments. */
export const stepNodePath = (missionId: string, stepPath: string): string =>
  [`mission[${missionId}]`, ...stepPath.split('/').map((part, index) => `${index % 2 === 0 ? 'step' : 'mission'}[${part}]`)].join('/')

// ---------- Frames ----------

export interface StripFrame { readonly file: number; readonly start: number; readonly end: number; readonly role: string; readonly precision: 'exact' | 'enclosing' }

const word = (value: OpenWord): string => typeof value === 'string' ? value : value.raw

const lineFrame = (span: SourceSpan, role: OpenWord, precision: OpenWord): StripFrame =>
  ({ file: span.file, start: span.start.line, end: span.end.line, role: word(role), precision: word(precision) === 'enclosing' ? 'enclosing' : 'exact' })

/** declaration › helper › value, outermost first; a node without frames is its own declaration. */
export const frameStrip = (node: SourceNode): readonly StripFrame[] =>
  node.frames.length === 0 ? [lineFrame(node.span, 'declaration', 'exact')] : node.frames.map(frame => lineFrame(frame, frame.role, frame.precision))

/** The frame shown first: the node's own authoring span when a frame names it, else the outermost. */
export const primaryFrame = (node: SourceNode, strip: readonly StripFrame[]): number => {
  const own = strip.findIndex(frame => frame.file === node.span.file && frame.start === node.span.start.line && frame.end === node.span.end.line)
  return own === -1 ? 0 : own
}

const ROLE_WORD: Record<string, string> = { declaration: 'declared', helper: 'via', fragment: 'via fragment', constructor: 'built by', value: 'value' }
export const roleWord = (role: string): string => ROLE_WORD[role] ?? role

export const basename = (path: string): string => path.slice(path.lastIndexOf('/') + 1)

/** `mission.ts:30–35`, or `in reviews.ts:3–19` for an enclosing range. */
export const frameLocation = (map: SourceMap, frame: StripFrame): string => {
  const name = basename(map.files[frame.file]?.path ?? '?')
  const lines = frame.start === frame.end ? `${frame.start}` : `${frame.start}–${frame.end}`
  return `${frame.precision === 'enclosing' ? 'in ' : ''}${name}:${lines}`
}

/** The top frame in the subject's own file, for chips: `mission.ts:30 · via review.ts`. */
export const chipLabel = (map: SourceMap, node: SourceNode): string => {
  const strip = frameStrip(node)
  const own = strip.find(frame => frame.file === map.declaration) ?? strip[0]!
  const via = strip.find(frame => frame !== own && frame.file !== own.file && (frame.role === 'helper' || frame.role === 'fragment'))
  const head = `${basename(map.files[own.file]?.path ?? '?')}:${own.start}`
  return via === undefined ? head : `${head} · via ${basename(map.files[via.file]?.path ?? '?')}`
}

/** A GitHub permalink at the revision's sha; only for files of the map's own repository, whose commit the map names. */
export const githubPermalink = (map: SourceMap, file: number, start?: number, end?: number): string | undefined => {
  const entry = map.files[file]
  if (entry === undefined || entry.repository !== map.repository) return undefined
  const repository = entry.repository.replace(/^https:\/\/github\.com\//, '').replace(/\.git$/, '')
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository)) return undefined
  const lines = start === undefined ? '' : end === undefined || end === start ? `#L${start}` : `#L${start}-L${end}`
  return `https://github.com/${repository}/blob/${map.sha}/${entry.path.split('/').map(encodeURIComponent).join('/')}${lines}`
}

// ---------- Linking ----------

export interface SourceLinks {
  readonly nodes: ReadonlyMap<string, SourceNode>
  /** For each file: every mapped range in it, as `[path, start, end]`. */
  readonly ranges: ReadonlyMap<number, readonly (readonly [string, number, number])[]>
}

export const sourceLinks = (map: SourceMap): SourceLinks => {
  const nodes = new Map(map.nodes.map(node => [node.kdl, node]))
  const ranges = new Map<number, (readonly [string, number, number])[]>()
  const add = (file: number, path: string, start: number, end: number) => {
    const list = ranges.get(file) ?? []
    list.push([path, start, end])
    ranges.set(file, list)
  }
  for (const node of map.nodes) {
    add(node.span.file, node.kdl, node.span.start.line, node.span.end.line)
    for (const frame of node.frames) if (frame.file !== node.span.file || frame.start.line !== node.span.start.line || frame.end.line !== node.span.end.line) add(frame.file, node.kdl, frame.start.line, frame.end.line)
  }
  return { nodes, ranges }
}

/** The node a TS line belongs to: the narrowest mapped range around it, the deeper node on a tie. */
export const nodeAtSourceLine = (links: SourceLinks, file: number, line: number): string | undefined => {
  let best: readonly [string, number, number] | undefined
  for (const range of links.ranges.get(file) ?? []) {
    if (range[1] > line || range[2] < line) continue
    if (best === undefined) { best = range; continue }
    const width = range[2] - range[1]
    const bestWidth = best[2] - best[1]
    if (width < bestWidth || (width === bestWidth && range[0].length > best[0].length)) best = range
  }
  return best?.[0]
}

export interface OutlineEntry { readonly path: string; readonly label: string; readonly depth: number; readonly location: string }

/** Declared nodes for the outline: the root, its direct children, and every agent/mission/step/gate. */
export const outline = (map: SourceMap, kdl: KdlIndex | undefined): readonly OutlineEntry[] => {
  const order = (path: string) => kdl?.byPath.get(path)?.start ?? Number.MAX_SAFE_INTEGER
  return map.nodes
    .map(node => ({ node, segments: nodeSegments(node.kdl) }))
    .map(({ node, segments }) => ({ node, depth: segments.length - 1, name: segments.at(-1)?.[0] ?? '' }))
    .filter(({ depth, name }) => depth <= 1 || named(name))
    .map(({ node, depth }, index) => ({ node, depth, index }))
    .sort((a, b) => order(a.node.kdl) - order(b.node.kdl) || a.index - b.index)
    .map(({ node, depth }) => {
      const strip = frameStrip(node)
      const own = strip.find(frame => frame.file === map.declaration) ?? strip[primaryFrame(node, strip)]!
      return { path: node.kdl, label: depth === 0 ? (nodeSegments(node.kdl)[0]?.[0] ?? node.kdl) : nodeLabel(node.kdl).split(' › ').at(-1)!, depth, location: frameLocation(map, own) }
    })
}

/** Files whose text the Source tab can show: every span and frame file, declaration first. */
export const sourceFiles = (map: SourceMap): readonly number[] => {
  const files = new Set<number>([map.declaration])
  for (const node of map.nodes) {
    files.add(node.span.file)
    for (const frame of node.frames) files.add(frame.file)
  }
  return [...files].filter(file => map.files[file] !== undefined)
}

// ---------- Diff ----------

export type DiffOp = { readonly kind: 'same' | 'add' | 'del'; readonly a?: number; readonly b?: number; readonly text: string }

/** Myers' O((N+M)D) line diff; `a`/`b` are 1-based line numbers. */
export const diffLines = (before: readonly string[], after: readonly string[]): readonly DiffOp[] => {
  const n = before.length
  const m = after.length
  const max = n + m
  const offset = max + 1
  const v = new Int32Array(2 * max + 3)
  const trace: Int32Array[] = []
  let found = false
  for (let d = 0; d <= max && !found; d += 1) {
    trace.push(v.slice())
    for (let k = -d; k <= d; k += 2) {
      let x = k === -d || (k !== d && v[offset + k - 1]! < v[offset + k + 1]!) ? v[offset + k + 1]! : v[offset + k - 1]! + 1
      let y = x - k
      while (x < n && y < m && before[x] === after[y]) { x += 1; y += 1 }
      v[offset + k] = x
      if (x >= n && y >= m) { found = true; break }
    }
  }
  const ops: DiffOp[] = []
  let x = n
  let y = m
  for (let d = trace.length - 1; d >= 0; d -= 1) {
    const vd = trace[d]!
    const k = x - y
    const previousK = k === -d || (k !== d && vd[offset + k - 1]! < vd[offset + k + 1]!) ? k + 1 : k - 1
    const previousX = vd[offset + previousK]!
    const previousY = previousX - previousK
    while (x > previousX && y > previousY) { ops.push({ kind: 'same', a: x, b: y, text: after[y - 1]! }); x -= 1; y -= 1 }
    if (d === 0) break
    if (x === previousX) { ops.push({ kind: 'add', b: y, text: after[y - 1]! }); y -= 1 }
    else { ops.push({ kind: 'del', a: x, text: before[x - 1]! }); x -= 1 }
  }
  return ops.reverse()
}

export type FoldedRow = DiffOp | { readonly kind: 'gap'; readonly folded: number }

/** Unified rows with `context` unchanged lines around each change; longer unchanged runs fold into a gap. */
export const foldContext = (ops: readonly DiffOp[], context = 3): readonly FoldedRow[] => {
  const keep = ops.map(() => false)
  ops.forEach((op, index) => { if (op.kind !== 'same') for (let near = Math.max(0, index - context); near <= Math.min(ops.length - 1, index + context); near += 1) keep[near] = true })
  const out: FoldedRow[] = []
  let folded = 0
  ops.forEach((op, index) => {
    if (keep[index]) {
      if (folded > 0) { out.push({ kind: 'gap', folded }); folded = 0 }
      out.push(op)
    } else folded += 1
  })
  if (folded > 0 && out.length > 0) out.push({ kind: 'gap', folded })
  return out
}

export const diffCounts = (ops: readonly DiffOp[]): { readonly added: number; readonly removed: number } =>
  ({ added: ops.filter(op => op.kind === 'add').length, removed: ops.filter(op => op.kind === 'del').length })

/** Display lines: one trailing newline ends the last line, it is not an extra empty line. */
export const textLines = (text: string): readonly string[] => (text.endsWith('\n') ? text.slice(0, -1) : text).split('\n')
export const diffTexts = (before: string, after: string): readonly DiffOp[] => diffLines(textLines(before), textLines(after))

// ---------- Changes between two revisions ----------

export interface FileChange {
  readonly key: string
  readonly repository: string
  readonly path: string
  readonly before?: string
  readonly after?: string
  readonly own: boolean
}

const fileKey = (file: SourceFile) => `${file.repository}\u0000${file.path}`

/** Contributing files whose text differs between two revisions, the subject's own file first. */
export const changedFiles = (before: SourceMap, after: SourceMap): readonly FileChange[] => {
  const left = new Map(before.files.map(file => [fileKey(file), file]))
  const right = new Map(after.files.map(file => [fileKey(file), file]))
  const own = new Set([fileKey(before.files[before.declaration]!), fileKey(after.files[after.declaration]!)].filter(Boolean))
  const changes: FileChange[] = []
  for (const key of new Set([...right.keys(), ...left.keys()])) {
    const a = left.get(key)
    const b = right.get(key)
    if (a?.blob_hash === b?.blob_hash) continue
    const file = (b ?? a)!
    changes.push({ key, repository: file.repository, path: file.path, ...(a === undefined ? {} : { before: a.blob_hash }), ...(b === undefined ? {} : { after: b.blob_hash }), own: own.has(key) })
  }
  return changes.sort((x, y) => Number(y.own) - Number(x.own) || x.path.localeCompare(y.path))
}

/** The changed KDL nodes, by the label of the nearest node around each changed line. */
export const changedNodes = (ops: readonly DiffOp[], beforeKdl: KdlIndex, afterKdl: KdlIndex): readonly string[] => {
  const labels = new Set<string>()
  for (const op of ops) {
    const path = op.kind === 'add' ? afterKdl.lineNode[op.b! - 1] : op.kind === 'del' ? beforeKdl.lineNode[op.a! - 1] : undefined
    if (path === undefined) continue
    // Name the change at step/gate level when there is one; fields read as `step x › goal`.
    labels.add(nodeLabel(path))
  }
  return [...labels]
}

/** One plain sentence: why the rendered KDL differs between two revisions. */
export const whyLine = ({ before, after, files, kdl }: {
  readonly before: SourceMap
  readonly after: SourceMap
  readonly files: readonly FileChange[]
  readonly kdl: { readonly added: number; readonly removed: number }
}): string => {
  const kind = after.subject.split('/')[0] ?? 'subject'
  const ownName = basename(after.files[after.declaration]?.path ?? before.files[before.declaration]?.path ?? 'its file')
  const own = files.some(file => file.own)
  const others = files.filter(file => !file.own)
  const noKdl = kdl.added === 0 && kdl.removed === 0
  if (files.length === 0) return noKdl ? 'No source file changed and the rendered KDL is identical.' : `No source file changed; the KDL differs because the rendering changed.`
  const tail = noKdl ? ' The rendered KDL is identical.' : ''
  if (own) return `The ${kind} file ${ownName} changed${others.length === 0 ? '' : `, plus ${others.length} ${others.length === 1 ? 'file' : 'files'} it uses`}.${tail}`
  const named = others.length === 1 ? basename(others[0]!.path) : `${others.length} files`
  return `The ${kind} file ${ownName} did not change. The change comes from ${named}, which this ${kind} uses.${tail}`
}

export const shortSha = (sha: string): string => sha.slice(0, 7)

const two = (value: number) => String(value).padStart(2, '0')

/** Picker label: `3f9c2ab1 · live — 3f9c2ab · seq 1911 · 11-10 09:25` (local time). */
export const revisionLabel = (revision: SourceRevision, live: string | undefined): string => {
  const at = new Date(revision.recorded_at_unix_ms)
  return `${revision.revision.slice(0, 8)}${revision.revision === live ? ' · live' : ''} — ${shortSha(revision.sha)} · seq ${revision.sequence} · ${two(at.getMonth() + 1)}-${two(at.getDate())} ${two(at.getHours())}:${two(at.getMinutes())}`
}
