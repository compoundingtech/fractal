/**
 * Source maps for rendered declarations: the `fractal.source-map/v1` document that
 * `fractal apply --set … --source-map FILE` records as one `source.mapped` claim per new subject
 * revision (docs/st3/owned-sets.md, "Source maps").
 *
 * Two passes locate each KDL node of a rendered subject in its TypeScript source:
 * - `sourceMap` walks the AST of the declaration module and its static import closure: the default
 *   export, its constructor call, the authoring object literal, its properties and array elements
 *   (through spreads of resolvable consts), and step objects by their `id` literal.
 * - `traceSourceReads` (Bun only) wraps every top-level literal `const` of the traced modules in a
 *   Proxy while the tree loads. Each primitive read records the property's span and the reading
 *   stack, which gives a node the frames declaration → helper → value. Values are returned
 *   unchanged, so the rendered KDL is byte-identical with and without the trace.
 *
 * The KDL node path is the join key (`mission[ID]/step[ID]/goal[0]`): agents, missions, steps and
 * gates are keyed by their first string argument, every other node by its index among siblings of
 * the same name. `kdlLines` are 1-based lines of the rendered text this map was built from.
 *
 * This module imports nothing outside Node. Callers pass `parseSync` from `oxc-parser`, so a
 * loader can use it from a pinned checkout without this package's dependencies.
 */
import { createHash } from 'node:crypto'
import { existsSync, readFileSync, realpathSync, statSync } from 'node:fs'
import { dirname, extname, resolve as resolvePath } from 'node:path'
import { fileURLToPath } from 'node:url'

/** A 1-based line and a 1-based column in UTF-16 code units, as editors count them. */
export interface Position {
  readonly line: number
  readonly column: number
}

/** A range in `files[file]`; `end` is exclusive. */
export interface Span {
  readonly file: number
  readonly start: Position
  readonly end: Position
}

/**
 * One step of how a node got its value, outermost first: the `declaration` call site in the
 * subject module, `helper` call sites, the `value` that was read, the `constructor` call that
 * builds the authoring object, or the `fragment` that declares a reused step. `enclosing` marks a
 * whole initializer rather than the exact property.
 */
export interface Frame extends Span {
  readonly role: 'declaration' | 'helper' | 'value' | 'constructor' | 'fragment'
  readonly precision: 'exact' | 'enclosing'
}

/** A KDL node with its own authoring site; unlisted nodes inherit the nearest listed ancestor. */
export interface SourceMapNode {
  readonly kdl: string
  readonly kdlLines: readonly [number, number]
  readonly span: Span
  readonly frames?: readonly Frame[]
}

/** A source file; `blob` is its git blob id, `sha256` the hash of `text` (UTF-8). */
export interface SourceMapFile {
  readonly repository: string
  readonly path: string
  readonly blob: string
  readonly sha256: string
  readonly text?: string
}

/** One subject: its declaration file, its static import closure (`inputs`) and its nodes. */
export interface SubjectSourceMap {
  readonly declaration: number
  readonly inputs: readonly number[]
  readonly nodes: readonly SourceMapNode[]
}

/** The publisher input of `fractal apply --source-map`. */
export interface SourceMapDocument {
  readonly schema: 'fractal.source-map/v1'
  readonly files: readonly SourceMapFile[]
  readonly subjects: Readonly<Record<string, SubjectSourceMap>>
}

/** Where a file lives: a repository (`owner/name`) and a POSIX path relative to its root. */
export interface FileLocation {
  readonly repository: string
  readonly path: string
}

/** A point in a module, by absolute path. */
export interface SourcePoint {
  readonly path: string
  readonly line: number
  readonly column: number
}

/**
 * A primitive read through a traced literal: the value, the span of the property that holds it,
 * and the traced call sites on the reading stack, innermost first.
 */
export interface SourceRead {
  readonly value: string | number | boolean
  readonly at: { readonly path: string; readonly start: Position; readonly end: Position }
  readonly stack: readonly SourcePoint[]
}

/** How much of the rendered KDL the map locates. */
export interface SourceMapCoverage {
  readonly subjects: number
  readonly files: number
  readonly kdlNodes: number
  /** Nodes with their own span. */
  readonly exactNodes: number
  /** Nodes without their own span below a node with one. */
  readonly enclosedNodes: number
  /** Nodes with neither; 0 when every subject's top node is located. */
  readonly orphanNodes: number
  /** Steps a mission declares, not `depends-on` references. */
  readonly declaredSteps: number
  readonly mappedSteps: number
  readonly framedNodes: number
  /** Nodes whose value a traced read located. */
  readonly tracedNodes: number
  readonly helperFramedNodes: number
}

/** The module shape `parseSync` from `oxc-parser` returns, as far as this module reads it. */
export interface ParsedModule {
  readonly program: unknown
  readonly module: {
    readonly staticImports: ReadonlyArray<{
      readonly moduleRequest: { readonly value: string }
      readonly entries: ReadonlyArray<{
        readonly localName: { readonly value: string }
        readonly importName: { readonly kind: string; readonly name?: string | null | undefined }
      }>
    }>
  }
  readonly errors: ReadonlyArray<{ readonly message: string }>
}

/** `parseSync` from `oxc-parser`; spans are UTF-16 offsets. */
export type Parse = (filename: string, sourceText: string) => ParsedModule

/** A source map that cannot be built; `path` is the file or subject at fault. */
export class SourceMapError extends Error {
  override readonly name = 'SourceMapError'
  readonly path: string
  constructor(path: string, message: string) {
    super(`${path}: ${message}`)
    this.path = path
  }
}

// ---------------------------------------------------------------------------------------------
// Text positions

const lineStarts = (text: string): readonly number[] => {
  const starts = [0]
  for (let index = 0; index < text.length; index++) {
    if (text.charCodeAt(index) === 10) starts.push(index + 1)
  }
  return starts
}

const positionAt = (starts: readonly number[], offset: number): Position => {
  let low = 0
  let high = starts.length - 1
  while (low < high) {
    const middle = (low + high + 1) >> 1
    if (starts[middle]! <= offset) low = middle
    else high = middle - 1
  }
  return { line: low + 1, column: offset - starts[low]! + 1 }
}

const offsetAt = (starts: readonly number[], point: Position): number =>
  (starts[point.line - 1] ?? 0) + point.column - 1

const gitBlob = (text: string): string => {
  const bytes = Buffer.from(text, 'utf8')
  return createHash('sha1').update(`blob ${bytes.length}\0`).update(bytes).digest('hex')
}

// ---------------------------------------------------------------------------------------------
// ESTree access

type AstNode = {
  readonly type: string
  readonly start: number
  readonly end: number
  readonly [key: string]: unknown
}

const isNode = (value: unknown): value is AstNode =>
  typeof value === 'object' &&
  value !== null &&
  'type' in value &&
  typeof value.type === 'string'

const field = (node: AstNode, key: string): AstNode | undefined => {
  const value = node[key]
  return isNode(value) === true ? value : undefined
}

const list = (node: AstNode, key: string): ReadonlyArray<AstNode | null> => {
  const value = node[key]
  return Array.isArray(value) === true ? (value as ReadonlyArray<AstNode | null>) : []
}

const wrappers: Readonly<Record<string, true>> = {
  TSAsExpression: true,
  TSSatisfiesExpression: true,
  TSNonNullExpression: true,
  TSTypeAssertion: true,
  ParenthesizedExpression: true,
}

/** The expression under type assertions and parentheses. */
const unwrap = (node: AstNode | undefined): AstNode | undefined =>
  node !== undefined && wrappers[node.type] === true
    ? unwrap(field(node, 'expression'))
    : node

/** A non-computed property's key, for identifier, string and number keys. */
const propertyName = (property: AstNode): string | undefined => {
  if (property.type !== 'Property' || property.computed === true) return undefined
  const key = field(property, 'key')
  if (key?.type === 'Identifier') return key.name as string
  if (key?.type === 'Literal' && (typeof key.value === 'string' || typeof key.value === 'number')) {
    return String(key.value)
  }
  return undefined
}

const isLiteralContainer = (node: AstNode | undefined): node is AstNode =>
  node?.type === 'ObjectExpression' || node?.type === 'ArrayExpression'

/** Every top-level `const NAME = INIT` (exported or not) whose binding is an identifier. */
const topLevelConsts = (program: AstNode): ReadonlyArray<{ name: string; init: AstNode }> =>
  list(program, 'body').flatMap((statement) => {
    if (statement === null) return []
    const declaration =
      statement.type === 'ExportNamedDeclaration' ? field(statement, 'declaration') : statement
    if (declaration?.type !== 'VariableDeclaration' || declaration.kind !== 'const') return []
    return list(declaration, 'declarations').flatMap((declarator) => {
      const id = declarator === null ? undefined : field(declarator, 'id')
      const init = declarator === null ? undefined : field(declarator, 'init')
      return id?.type === 'Identifier' && init !== undefined
        ? [{ name: id.name as string, init }]
        : []
    })
  })

/** Visits every node below `root`, depth first. */
const walk = (root: AstNode, visit: (node: AstNode) => void): void => {
  visit(root)
  for (const key of Object.keys(root)) {
    const value = root[key]
    if (Array.isArray(value) === true) {
      for (const item of value as unknown[]) if (isNode(item) === true) walk(item, visit)
    } else if (isNode(value) === true) walk(value, visit)
  }
}

// ---------------------------------------------------------------------------------------------
// Rendered KDL

interface KdlNode {
  readonly name: string
  readonly arg: string | undefined
  readonly path: string
  readonly line: number
  end: number
  readonly children: KdlNode[]
}

/** Nodes keyed by their first string argument instead of their sibling index. */
const keyedNodes: Readonly<Record<string, true>> = { agent: true, mission: true, step: true, gate: true }

const quoted = (text: string, at: number, path: string, line: number) => {
  let index = at + 1
  while (index < text.length && text[index] !== '"') index += text[index] === '\\' ? 2 : 1
  if (index >= text.length) throw new SourceMapError(path, `KDL line ${line} has an open string`)
  return { value: JSON.parse(text.slice(at, index + 1)) as string, end: index + 1 }
}

/**
 * Reads the canonical emitter's output: one node per line, `}` on its own line, strings in JSON
 * syntax. `subject` names the text in errors.
 */
const scanKdl = (kdl: string, subject: string): readonly KdlNode[] => {
  const roots: KdlNode[] = []
  const stack: KdlNode[] = []
  kdl.split('\n').forEach((raw, index) => {
    const lineNumber = index + 1
    const line = raw.trim()
    if (line === '') return
    if (line === '}') {
      const closed = stack.pop()
      if (closed === undefined) throw new SourceMapError(subject, `KDL line ${lineNumber} closes nothing`)
      closed.end = lineNumber
      return
    }
    const bare = /^[^\s{]+/u.exec(line)![0]
    const name = line.startsWith('"')
      ? quoted(line, 0, subject, lineNumber)
      : { value: bare, end: bare.length }
    const rest = line.slice(name.end).trimStart()
    const arg = rest.startsWith('"') ? quoted(rest, 0, subject, lineNumber).value : undefined
    const parent = stack.at(-1)
    const siblings = parent === undefined ? roots : parent.children
    const keyed = keyedNodes[name.value] === true && arg !== undefined
    const named = keyed === true ? `${name.value}[${arg}]` : undefined
    const prefix = parent === undefined ? '' : `${parent.path}/`
    const duplicate = named !== undefined && siblings.some((node) => node.path === prefix + named)
    const key =
      named !== undefined && duplicate === false
        ? named
        : `${name.value}[${siblings.filter((node) => node.name === name.value).length}]`
    const node: KdlNode = {
      name: name.value,
      arg,
      path: prefix + key,
      line: lineNumber,
      end: lineNumber,
      children: [],
    }
    siblings.push(node)
    if (line.endsWith('{') === true) stack.push(node)
  })
  if (stack.length > 0) throw new SourceMapError(subject, 'KDL ends inside a children block')
  return roots
}

// ---------------------------------------------------------------------------------------------
// Mapping

interface SourceFile {
  readonly path: string
  readonly text: string
  readonly program: AstNode
  readonly starts: readonly number[]
  readonly imports: ReadonlyArray<{
    readonly source: string
    readonly names: ReadonlyArray<{ readonly local: string; readonly imported: string }>
  }>
}

type FrameDraft = Omit<Frame, 'precision'> & { readonly precision?: Frame['precision'] }

interface Hit {
  readonly span: Span
  readonly frames: readonly FrameDraft[]
  /** The literal the AST span authors, when it is one (`goal: 'text'`). */
  readonly literal?: unknown
}

const moduleExtensions: Readonly<Record<string, true>> = {
  '.ts': true,
  '.tsx': true,
  '.mts': true,
  '.cts': true,
  '.js': true,
  '.mjs': true,
  '.cjs': true,
  '.jsx': true,
}

/**
 * Builds the source map of rendered subjects.
 *
 * - `subjects`: each subject (`agent/ID` or `mission/ID`), its declaration module's absolute path
 *   and the KDL text that is published.
 * - `locate`: the repository and repository-relative path of a file; files it does not locate are
 *   neither inputs nor frames (dependencies installed from a registry, for example).
 * - `resolve`: the file a bare import specifier resolves to from a module; relative specifiers
 *   resolve against the importing file.
 * - `reads`: the reads `traceSourceReads` recorded while the same modules loaded.
 * - `texts`: include each file's text, which a daemon needs unless it already stores the blob.
 */
export const sourceMap = ({
  parse,
  subjects,
  locate,
  resolve,
  reads = [],
  texts = false,
}: {
  readonly parse: Parse
  readonly subjects: ReadonlyArray<{
    readonly subject: string
    readonly file: string
    readonly kdl: string
  }>
  readonly locate: (file: string) => FileLocation | undefined
  readonly resolve?: (specifier: string, from: string) => string | undefined
  readonly reads?: readonly SourceRead[]
  readonly texts?: boolean
}): { readonly document: SourceMapDocument; readonly coverage: SourceMapCoverage } => {
  const parsed = new Map<string, SourceFile | undefined>()
  const files: SourceMapFile[] = []
  const fileIds = new Map<string, number>()

  const load = (path: string): SourceFile | undefined => {
    if (parsed.has(path) === true) return parsed.get(path)
    const module = moduleExtensions[extname(path)] === true && path.endsWith('.d.ts') === false
    if (module === false || locate(path) === undefined || existsSync(path) === false) {
      parsed.set(path, undefined)
      return undefined
    }
    const text = readFileSync(path, 'utf8')
    const result = parse(path, text)
    if (result.errors.length > 0) {
      throw new SourceMapError(path, `cannot parse: ${result.errors[0]!.message}`)
    }
    const file: SourceFile = {
      path,
      text,
      program: result.program as AstNode,
      starts: lineStarts(text),
      imports: result.module.staticImports.map((entry) => ({
        source: entry.moduleRequest.value,
        names: entry.entries.map((name) => ({
          local: name.localName.value,
          imported: name.importName.name ?? name.importName.kind,
        })),
      })),
    }
    parsed.set(path, file)
    return file
  }

  const fileId = (file: SourceFile): number => {
    const known = fileIds.get(file.path)
    if (known !== undefined) return known
    const location = locate(file.path)!
    files.push({
      ...location,
      blob: gitBlob(file.text),
      sha256: createHash('sha256').update(file.text, 'utf8').digest('hex'),
      ...(texts === true ? { text: file.text } : {}),
    })
    fileIds.set(file.path, files.length - 1)
    return files.length - 1
  }

  const spanOf = (file: SourceFile, node: { readonly start: number; readonly end: number }): Span => ({
    file: fileId(file),
    start: positionAt(file.starts, node.start),
    end: positionAt(file.starts, node.end),
  })

  const resolveImport = (from: string, specifier: string): string | undefined => {
    if (specifier.startsWith('.') === false) return resolve?.(specifier, from)
    const base = resolvePath(dirname(from), specifier)
    for (const candidate of [base, `${base}.ts`, `${base}/index.ts`]) {
      if (existsSync(candidate) === true && statSync(candidate).isFile() === true) return candidate
    }
    return undefined
  }

  const importClosure = (root: SourceFile): readonly SourceFile[] => {
    const seen = new Map<string, SourceFile>([[root.path, root]])
    const queue = [root]
    while (queue.length > 0) {
      const file = queue.pop()!
      for (const entry of file.imports) {
        const target = resolveImport(file.path, entry.source)
        if (target === undefined || seen.has(target) === true) continue
        const other = load(target)
        if (other === undefined) continue
        seen.set(target, other)
        queue.push(other)
      }
    }
    return [...seen.values()]
  }

  /** An identifier's defining expression: a same-file const or a statically imported one. */
  const resolveIdentifier = (
    file: SourceFile,
    name: string,
  ): { readonly file: SourceFile; readonly node: AstNode } | undefined => {
    const local = topLevelConsts(file.program).find((entry) => entry.name === name)
    if (local !== undefined) return { file, node: unwrap(local.init)! }
    for (const entry of file.imports) {
      const imported = entry.names.find((candidate) => candidate.local === name)
      if (imported === undefined) continue
      const target = resolveImport(file.path, entry.source)
      const other = target === undefined ? undefined : load(target)
      const init =
        other === undefined
          ? undefined
          : topLevelConsts(other.program).find((candidate) => candidate.name === imported.imported)
      if (other !== undefined && init !== undefined) return { file: other, node: unwrap(init.init)! }
    }
    return undefined
  }

  /** `define*({ meta, intent })` forms author the declaration in `intent`, maybe as a function. */
  const intentOf = (object: AstNode): AstNode => {
    const intent = list(object, 'properties').find(
      (property) => property !== null && propertyName(property) === 'intent',
    )
    let value = intent === undefined || intent === null ? undefined : unwrap(field(intent, 'value'))
    if (value?.type === 'ArrowFunctionExpression' || value?.type === 'FunctionExpression') {
      const body = field(value, 'body')
      value =
        body?.type === 'BlockStatement'
          ? unwrap(
              list(body, 'body')
                .filter((statement) => statement?.type === 'ReturnStatement')
                .map((statement) => field(statement!, 'argument'))
                .at(-1),
            )
          : unwrap(body)
    }
    return value?.type === 'ObjectExpression' ? value : object
  }

  /** The object literal that authors a declaration, through constructor calls and consts. */
  const objectOf = (
    file: SourceFile,
    expression: AstNode | undefined,
    frames: FrameDraft[],
    depth = 0,
  ): { readonly file: SourceFile; readonly node: AstNode } | undefined => {
    const node = unwrap(expression)
    if (node === undefined || depth > 4) return undefined
    if (node.type === 'ObjectExpression') return { file, node: intentOf(node) }
    if (node.type === 'CallExpression') {
      const argument = list(node, 'arguments')[0]
      if (argument === undefined || argument === null) return undefined
      frames.push({ ...spanOf(file, node), role: 'constructor' })
      return objectOf(file, argument, frames, depth + 1)
    }
    if (node.type === 'Identifier') {
      const target = resolveIdentifier(file, node.name as string)
      if (target === undefined) return undefined
      if (target.file !== file) frames.push({ ...spanOf(target.file, target.node), role: 'value' })
      return objectOf(target.file, target.node, frames, depth + 1)
    }
    if (node.type === 'MemberExpression') {
      const object = field(node, 'object')
      if (object?.type !== 'Identifier') return undefined
      const target = resolveIdentifier(file, object.name as string)
      return target === undefined ? undefined : objectOf(target.file, target.node, frames, depth + 1)
    }
    return undefined
  }

  /** Array elements through spreads of resolvable consts, each with its own file. */
  const elements = (
    file: SourceFile,
    expression: AstNode | undefined,
    depth = 0,
  ): ReadonlyArray<{ readonly file: SourceFile; readonly node: AstNode; readonly via?: FrameDraft }> => {
    const node = unwrap(expression)
    if (node === undefined || depth > 4) return []
    if (node.type === 'Identifier') {
      const target = resolveIdentifier(file, node.name as string)
      if (target === undefined) return []
      const via: FrameDraft | undefined =
        target.file !== file ? { ...spanOf(target.file, target.node), role: 'value' } : undefined
      return elements(target.file, target.node, depth + 1).map((element) =>
        element.via === undefined && via !== undefined ? { ...element, via } : element,
      )
    }
    if (node.type !== 'ArrayExpression') return []
    return list(node, 'elements').flatMap((element) => {
      if (element === null) return []
      if (element.type === 'SpreadElement') {
        return elements(file, field(element, 'argument'), depth + 1)
      }
      return [{ file, node: element }]
    })
  }

  const kebab = (name: string) => name.replace(/[A-Z]/gu, (letter) => `-${letter.toLowerCase()}`)

  const literalOf = (node: AstNode | undefined): unknown => {
    const value = unwrap(node)
    return value?.type === 'Literal' ? value.value : undefined
  }

  const stepId = (object: AstNode): unknown =>
    literalOf(
      list(object, 'properties')
        .filter((property) => property !== null && propertyName(property) === 'id')
        .map((property) => field(property!, 'value'))[0],
    )

  /** Maps the KDL children of `kdl` onto the properties of its authoring object. */
  const mapFields = (
    kdl: KdlNode,
    object: { readonly file: SourceFile; readonly node: AstNode },
    out: Map<string, Hit>,
  ): void => {
    for (const property of list(object.node, 'properties')) {
      if (property === null) continue
      const name = propertyName(property)
      if (name === undefined) continue
      const child = kebab(name)
      const singular = child.endsWith('s') === true ? child.slice(0, -1) : child
      const kids = kdl.children.filter((kid) => kid.name === child || kid.name === singular)
      if (kids.length === 0) continue
      const value = unwrap(field(property, 'value'))
      if (name === 'steps') {
        for (const element of elements(object.file, value)) {
          const frames: FrameDraft[] = element.via === undefined ? [] : [element.via]
          const step = objectOf(element.file, element.node, frames)
          if (step === undefined) continue
          const kid = kdl.children.find((candidate) => candidate.name === 'step' && candidate.arg === stepId(step.node))
          if (kid === undefined) continue
          out.set(kid.path, { span: spanOf(step.file, step.node), frames })
          mapFields(kid, step, out)
        }
        continue
      }
      const items =
        value?.type === 'ArrayExpression' || value?.type === 'Identifier'
          ? elements(object.file, value)
          : []
      if (items.length > 0 && items.length === kids.length) {
        items.forEach((item, index) =>
          out.set(kids[index]!.path, {
            span: spanOf(item.file, item.node),
            frames: item.via === undefined ? [] : [item.via],
            literal: literalOf(item.node),
          }),
        )
      } else if (kids.length === 1) {
        const frames: FrameDraft[] = []
        if (value?.type === 'Identifier') {
          const target = resolveIdentifier(object.file, value.name as string)
          if (target !== undefined && target.file !== object.file) {
            frames.push({ ...spanOf(target.file, target.node), role: 'value' })
          }
        }
        out.set(kids[0]!.path, { span: spanOf(object.file, property), frames, literal: literalOf(value) })
        if (value?.type === 'ObjectExpression') mapFields(kids[0]!, { file: object.file, node: value }, out)
      } else {
        for (const kid of kids) out.set(kid.path, { span: spanOf(object.file, property), frames: [] })
      }
    }
  }

  /** Steps not reached from the declaration (built by helpers): `id` literals in the closure. */
  const fragmentSteps = (top: KdlNode, closure: readonly SourceFile[], out: Map<string, Hit>): void => {
    const declared: KdlNode[] = []
    const collect = (node: KdlNode) => {
      for (const kid of node.children) {
        if (kid.name === 'depends-on') continue
        if (kid.name === 'step' && out.has(kid.path) === false) declared.push(kid)
        collect(kid)
      }
    }
    collect(top)
    for (const kid of declared) {
      for (const file of closure) {
        let found: AstNode | undefined
        walk(file.program, (node) => {
          if (found === undefined && node.type === 'ObjectExpression' && stepId(node) === kid.arg) found = node
        })
        if (found === undefined) continue
        out.set(kid.path, { span: spanOf(file, found), frames: [{ ...spanOf(file, found), role: 'fragment' }] })
        mapFields(kid, { file, node: found }, out)
        break
      }
    }
  }

  // Reads belong to the subject whose module is the outermost declaration on their stack.
  const declarationFiles = new Set(subjects.map((subject) => subject.file))
  const readsByFile = new Map<string, SourceRead[]>()
  for (const read of reads) {
    const owner = read.stack.findLast((point) => declarationFiles.has(point.path) === true)
    if (owner === undefined) continue
    readsByFile.set(owner.path, [...(readsByFile.get(owner.path) ?? []), read])
  }

  const pointFrame = (point: SourcePoint, role: Frame['role']): Frame | undefined => {
    const file = load(point.path)
    if (file === undefined) return undefined
    const at = { line: point.line, column: point.column }
    return { file: fileId(file), start: at, end: at, role, precision: 'exact' }
  }

  /** declaration (the subject's call site) → helpers → value, outermost first. */
  const readFrames = (read: SourceRead, declaration: string): readonly Frame[] => {
    const outer = read.stack.findLastIndex((point) => point.path === declaration)
    const frames: Frame[] = []
    const seen = new Set<string>()
    for (const [index, point] of read.stack.slice(0, outer + 1).toReversed().entries()) {
      if (seen.has(`${point.path}:${point.line}`) === true) continue
      seen.add(`${point.path}:${point.line}`)
      const frame = pointFrame(point, index === 0 ? 'declaration' : 'helper')
      if (frame !== undefined) frames.push(frame)
    }
    const valueFile = load(read.at.path)
    if (valueFile !== undefined) {
      frames.push({ file: fileId(valueFile), start: read.at.start, end: read.at.end, role: 'value', precision: 'exact' })
    }
    return frames
  }

  /** The innermost `step({…})` call around a helper's call site. */
  const stepCallAround = (file: SourceFile, point: SourcePoint): AstNode | undefined => {
    const offset = offsetAt(file.starts, point)
    let call: AstNode | undefined
    walk(file.program, (node) => {
      if (node.type !== 'CallExpression' || node.start > offset || offset > node.end) return
      const callee = field(node, 'callee')
      const name =
        callee?.type === 'Identifier'
          ? callee.name
          : callee?.type === 'MemberExpression'
            ? field(callee, 'property')?.name
            : undefined
      if (name === 'step' && (call === undefined || node.start >= call.start)) call = node
    })
    return call
  }

  let kdlNodes = 0
  let exactNodes = 0
  let enclosedNodes = 0
  let orphanNodes = 0
  let declaredSteps = 0
  let mappedSteps = 0
  let framedNodes = 0
  let tracedNodes = 0
  let helperFramedNodes = 0
  const maps: Record<string, SubjectSourceMap> = {}

  for (const subject of subjects) {
    const root = load(subject.file)
    if (root === undefined) {
      throw new SourceMapError(subject.subject, `declaration ${subject.file} is not a located module`)
    }
    const top = scanKdl(subject.kdl, subject.subject).find((node) => node.name !== 'version')
    if (top === undefined) throw new SourceMapError(subject.subject, 'KDL declares no subject node')
    const closure = importClosure(root)
    const out = new Map<string, Hit>()
    const exported = list(root.program, 'body').find((statement) => statement?.type === 'ExportDefaultDeclaration')
    if (exported !== undefined && exported !== null) {
      const frames: FrameDraft[] = []
      const object = objectOf(root, field(exported, 'declaration'), frames)
      // The outermost constructor call (`makeSeat({…})`) is the declaration, not `export default x`.
      const constructor = frames.find((frame) => frame.role === 'constructor')
      out.set(top.path, { span: constructor ?? spanOf(root, exported), frames })
      if (object !== undefined) mapFields(top, object, out)
    }
    fragmentSteps(top, closure, out)

    const traced = readsByFile.get(subject.file) ?? []
    const rootId = fileId(root)
    /** The read's outermost call site in the declaration module. */
    const siteOf = (read: SourceRead) => read.stack.findLast((point) => point.path === subject.file)
    const within = (span: Span, point: SourcePoint): boolean => {
      const after = point.line > span.start.line || (point.line === span.start.line && point.column >= span.start.column)
      const before = point.line < span.end.line || (point.line === span.end.line && point.column < span.end.column)
      return after === true && before === true
    }
    /**
     * The reads authored at one call site: inside `site` (the node's own span in the declaration
     * module, else its nearest ancestor's) when any are, else anywhere (a module constant read
     * before the declaration). Several sites — a helper called twice — are ambiguous.
     */
    const bound = (candidates: readonly SourceRead[], site: Span | undefined): readonly SourceRead[] => {
      const inside = candidates.filter((candidate) => {
        const point = siteOf(candidate)
        return site !== undefined && point !== undefined && within(site, point) === true
      })
      const groups = new Map<string, SourceRead[]>()
      for (const candidate of inside.length > 0 ? inside : candidates) {
        const point = siteOf(candidate)
        const key = point === undefined ? '' : `${point.line}:${point.column}`
        groups.set(key, [...(groups.get(key) ?? []), candidate])
      }
      return groups.size === 1 ? [...groups.values()][0]! : []
    }
    /** The innermost node of `type` in the declaration module around `point`. */
    const innermost = (type: string, point: SourcePoint): AstNode | undefined => {
      const offset = offsetAt(root.starts, point)
      let found: AstNode | undefined
      walk(root.program, (node) => {
        if (node.type !== type || node.start > offset || offset > node.end) return
        if (found === undefined || node.start >= found.start) found = node
      })
      return found
    }
    /** The object literal whose own `id` property value holds `point` (`step({ id: ids.second })`). */
    const keyedObjectAround = (point: SourcePoint): Span | undefined => {
      const object = innermost('ObjectExpression', point)
      if (object === undefined) return undefined
      const offset = offsetAt(root.starts, point)
      const id = list(object, 'properties').find((property) => {
        const key = property === null ? undefined : field(property, 'key')
        return key?.type === 'Identifier' && key.name === 'id'
      })
      const value = id === undefined || id === null ? undefined : field(id, 'value')
      return value !== undefined && value.start <= offset && offset <= value.end ? spanOf(root, object) : undefined
    }
    /**
     * A keyed node a helper builds (`reviewStep('code', 'one')`): the one call in the declaration
     * module that passes its key as a literal argument bounds the reads authored for it.
     */
    const keyCall = (node: KdlNode): Span | undefined => {
      if (keyedNodes[node.name] !== true || node.arg === undefined) return undefined
      const calls: AstNode[] = []
      walk(root.program, (candidate) => {
        if (candidate.type !== 'CallExpression') return
        if (list(candidate, 'arguments').some((argument) => argument?.type === 'Literal' && argument.value === node.arg)) calls.push(candidate)
      })
      return calls.length === 1 ? spanOf(root, calls[0]!) : undefined
    }
    const nodes: SourceMapNode[] = []
    const visit = (node: KdlNode, enclosed: boolean, reference: boolean, outer: Span | undefined) => {
      kdlNodes++
      const declaredStep = node.name === 'step' && reference === false
      if (declaredStep === true) declaredSteps++
      let hit = out.get(node.path)
      let site = hit !== undefined && hit.span.file === rootId ? hit.span : (keyCall(node) ?? outer)
      // A keyed node's span is its declaration; a traced read of its key only adds frames.
      const keyed = keyedNodes[node.name] === true
      const candidates =
        node.arg === undefined || (hit !== undefined && hit.literal === node.arg)
          ? []
          : bound(traced.filter((candidate) => candidate.value === node.arg), site)
      const distinct = new Set(candidates.map((candidate) => `${candidate.at.path}:${candidate.at.start.line}:${candidate.at.start.column}`))
      let read = distinct.size === 1 ? candidates[0] : undefined
      // A value copied out of another traced literal (`{ push: rules.push }`): follow it to its author.
      for (let depth = 0; read !== undefined && read.at.path === subject.file && depth < traced.length; depth++) {
        const { at: from, value } = read
        const copied = traced.find((candidate) => {
          const point = siteOf(candidate)
          return candidate !== read && candidate.value === value && point !== undefined &&
            within({ file: rootId, start: from.start, end: from.end }, point) === true
        })
        if (copied === undefined) break
        read = copied
      }
      if (read !== undefined) {
        const frames = readFrames(read, subject.file)
        const value = frames.at(-1)
        const readSite = siteOf(read)
        const declared = keyed === false ? undefined : (hit?.span ?? (readSite === undefined ? undefined : keyedObjectAround(readSite)))
        if (keyed === true && declared === undefined && readSite !== undefined) {
          // The key passed to a helper (`reviewStep('code', ids.second)`): that call bounds the
          // node's reads, and the helper-step branch below locates the node.
          const call = innermost('CallExpression', readSite)
          if (call !== undefined) site = spanOf(root, call)
        } else if (value !== undefined && value.role === 'value' && (keyed === false || declared !== undefined)) {
          tracedNodes++
          if (frames.some((frame) => frame.role === 'helper') === true) helperFramedNodes++
          hit = {
            span: declared ?? { file: value.file, start: value.start, end: value.end },
            frames: [...(hit?.frames ?? []).filter((frame) => frame.role !== 'value'), ...frames],
          }
        }
      }
      if (hit === undefined && declaredStep === true) {
        // A step built by a helper: a read beneath it names the helper; its `step({…})` is the span.
        const values = new Set<unknown>()
        const collect = (kid: KdlNode) => {
          if (kid.arg !== undefined) values.add(kid.arg)
          kid.children.forEach(collect)
        }
        node.children.forEach(collect)
        const beneath = bound(
          traced.filter(
            (candidate) => values.has(candidate.value) === true && candidate.stack.some((point) => point.path !== subject.file),
          ),
          site,
        )[0]
        const helper = beneath?.stack.find((point) => point.path !== subject.file)
        const helperFile = helper === undefined ? undefined : load(helper.path)
        const call = helper === undefined || helperFile === undefined ? undefined : stepCallAround(helperFile, helper)
        const declarationSite = beneath === undefined ? undefined : siteOf(beneath)
        if (call !== undefined && declarationSite !== undefined && helperFile !== undefined) {
          const declaration = pointFrame(declarationSite, 'declaration')!
          hit = {
            span: spanOf(helperFile, call),
            frames: [declaration, { ...spanOf(helperFile, call), role: 'helper', precision: 'exact' }],
          }
        }
      }
      if (hit !== undefined) {
        exactNodes++
        if (declaredStep === true) mappedSteps++
        if (hit.frames.length > 0) framedNodes++
        const frames = hit.frames.map(
          (frame): Frame => ({
            ...frame,
            precision: frame.precision ?? (frame.role === 'value' ? 'enclosing' : 'exact'),
          }),
        )
        nodes.push({
          kdl: node.path,
          kdlLines: [node.line, node.end],
          span: { file: hit.span.file, start: hit.span.start, end: hit.span.end },
          ...(frames.length > 0 ? { frames } : {}),
        })
      } else if (enclosed === true) enclosedNodes++
      else orphanNodes++
      for (const kid of node.children) {
        visit(kid, enclosed === true || hit !== undefined, reference === true || node.name === 'depends-on', site)
      }
    }
    visit(top, false, false, undefined)
    maps[subject.subject] = {
      declaration: fileId(root),
      inputs: closure.map(fileId).toSorted((a, b) => a - b),
      nodes,
    }
  }

  return {
    document: { schema: 'fractal.source-map/v1', files, subjects: maps },
    coverage: {
      subjects: subjects.length,
      files: files.length,
      kdlNodes,
      exactNodes,
      enclosedNodes,
      orphanNodes,
      declaredSteps,
      mappedSteps,
      framedNodes,
      tracedNodes,
      helperFramedNodes,
    },
  }
}

// ---------------------------------------------------------------------------------------------
// Runtime read trace (Bun)

interface BunRuntime {
  readonly plugin: (plugin: {
    readonly name: string
    readonly setup: (build: {
      readonly onLoad: (
        options: { readonly filter: RegExp },
        callback: (args: { readonly path: string }) => { contents: string; loader: string },
      ) => void
    }) => void
  }) => void
}

const wrapGlobal = '__fractalSourceRead'
const thisModule = fileURLToPath(import.meta.url)
const loaders: Readonly<Record<string, string>> = { '.ts': 'ts', '.mts': 'ts', '.cts': 'ts', '.tsx': 'tsx' }

/** `[start, end, nested]` of each access path (`policy.reviews.code.goal`) inside one literal. */
type AccessTable = Record<string, readonly [number, number, boolean]>

const accessTable = (node: AstNode, prefix: string, out: AccessTable): void => {
  const entries: ReadonlyArray<readonly [string, AstNode]> =
    node.type === 'ObjectExpression'
      ? list(node, 'properties').flatMap((property) => {
          const name = property === null ? undefined : propertyName(property)
          return name === undefined ? [] : [[name, property!] as const]
        })
      : list(node, 'elements').flatMap((element, index) =>
          element === null || element.type === 'SpreadElement' ? [] : [[String(index), element] as const],
        )
  for (const [key, site] of entries) {
    const value = unwrap(site.type === 'Property' ? field(site, 'value') : site)
    const nested = isLiteralContainer(value)
    out[prefix + key] = [site.start, site.end, nested]
    if (nested === true) accessTable(value, `${prefix}${key}.`, out)
  }
}

/**
 * Records primitive reads through the top-level literal consts of the modules below `roots`
 * (absolute directories) that Bun loads after this call; `node_modules` below a root stay
 * untouched. Install it once per process, before the tree is imported, and pass `reads()` to
 * `sourceMap`.
 *
 * Guardrails: only literal object and array initializers are wrapped, never frozen values or
 * non-writable properties, and a nested value only when its initializer is itself a literal, so
 * step and gate handles keep their identity. A nested literal is always the same Proxy.
 */
export const traceSourceReads = ({
  parse,
  roots,
}: {
  readonly parse: Parse
  readonly roots: readonly string[]
}): { readonly reads: () => readonly SourceRead[] } => {
  // Bun's global is untyped here: this package compiles against Node's types only.
  const bun = (globalThis as { readonly Bun?: BunRuntime }).Bun
  if (bun === undefined) throw new SourceMapError(thisModule, 'traceSourceReads runs only in Bun')
  const host = globalThis as Record<string, unknown>
  if (host[wrapGlobal] !== undefined) {
    throw new SourceMapError(thisModule, 'traceSourceReads is already installed in this process')
  }
  const realRoots = roots.map((root) => realpathSync(root))
  const reads: SourceRead[] = []
  const modules: Array<{
    readonly path: string
    readonly starts: readonly number[]
    readonly table: AccessTable
    /** Inserted text per line as `[original column, length]`, ascending. */
    readonly insertions: ReadonlyMap<number, ReadonlyArray<readonly [number, number]>>
  }> = []
  const traced = new Map<string, number>()

  /** A stack point in original coordinates: the trace inserts text into the modules it loads. */
  const original = (path: string, line: number, column: number): SourcePoint => {
    const id = traced.get(path)
    let shift = 0
    for (const [at, length] of (id === undefined ? undefined : modules[id]!.insertions.get(line)) ?? []) {
      if (column >= at + shift + length) shift += length
      else break
    }
    return { path, line, column: column - shift }
  }

  const stack = (): readonly SourcePoint[] => {
    const limit = Error.stackTraceLimit
    Error.stackTraceLimit = Number.POSITIVE_INFINITY
    const text = new Error().stack ?? ''
    Error.stackTraceLimit = limit
    const points: SourcePoint[] = []
    for (const line of text.split('\n').slice(1)) {
      const match = /\(?(?:file:\/\/)?(\/[^()\s]*?):(\d+):(\d+)\)?$/u.exec(line.trim())
      if (match === null) continue
      const path = decodeURIComponent(match[1]!)
      if (path === thisModule || traced.has(path) === false) continue
      points.push(original(path, Number(match[2]), Number(match[3])))
    }
    return points
  }

  const proxies = new WeakMap<object, object>()
  const wrap = (value: unknown, id: number, prefix: string): unknown => {
    if (value === null || typeof value !== 'object' || Object.isFrozen(value) === true) return value
    const known = proxies.get(value)
    if (known !== undefined) return known
    const proxy = new Proxy(value, {
      get(target, key, receiver) {
        const result: unknown = Reflect.get(target, key, receiver)
        if (typeof key !== 'string') return result
        const module = modules[id]!
        const entry = module.table[prefix + key]
        if (entry === undefined) return result
        const descriptor = Reflect.getOwnPropertyDescriptor(target, key)
        if (descriptor !== undefined && descriptor.configurable === false && descriptor.writable === false) {
          return result
        }
        if (entry[2] === true) return wrap(result, id, `${prefix}${key}.`)
        if (typeof result === 'string' || typeof result === 'number' || typeof result === 'boolean') {
          reads.push({
            value: result,
            at: {
              path: module.path,
              start: positionAt(module.starts, entry[0]),
              end: positionAt(module.starts, entry[1]),
            },
            stack: stack(),
          })
        }
        return result
      },
    })
    proxies.set(value, proxy)
    return proxy
  }
  host[wrapGlobal] = wrap

  const escaped = realRoots.map((root) => root.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&'))
  bun.plugin({
    name: 'fractal-source-read-trace',
    setup(build) {
      build.onLoad({ filter: new RegExp(`^(?:${escaped.join('|')})/.*\\.[cm]?tsx?$`, 'u') }, ({ path }) => {
        const text = readFileSync(path, 'utf8')
        const loader = loaders[extname(path)] ?? 'ts'
        const root = realRoots.find((candidate) => path.startsWith(`${candidate}/`))
        if (root === undefined || path.slice(root.length).includes('/node_modules/') === true) {
          return { contents: text, loader }
        }
        const result = parse(path, text)
        if (result.errors.length > 0) return { contents: text, loader }
        const id = modules.length
        const table: AccessTable = {}
        const edits: Array<readonly [number, string]> = []
        for (const { name, init } of topLevelConsts(result.program as AstNode)) {
          if (isLiteralContainer(unwrap(init)) === false) continue
          accessTable(unwrap(init)!, `${name}.`, table)
          edits.push([init.start, `globalThis.${wrapGlobal}(`], [init.end, `, ${id}, ${JSON.stringify(`${name}.`)})`])
        }
        const starts = lineStarts(text)
        const insertions = new Map<number, Array<readonly [number, number]>>()
        for (const [offset, inserted] of edits) {
          const at = positionAt(starts, offset)
          insertions.set(at.line, [...(insertions.get(at.line) ?? []), [at.column, inserted.length] as const])
        }
        modules.push({ path, starts, table, insertions })
        traced.set(path, id)
        let contents = text
        for (const [offset, inserted] of edits.toSorted((a, b) => b[0] - a[0])) {
          contents = contents.slice(0, offset) + inserted + contents.slice(offset)
        }
        return { contents, loader }
      })
    },
  })
  return { reads: () => reads }
}
