import { Agent, Resource, UsageSummary, decodeUnknownSync } from '@smalltalk/st3-client/schema'
import { Schema, SchemaAST, SchemaTransformation } from 'effect'
import type { Brand, DateTime, Duration, Option } from 'effect'
import type { Issue } from 'effect/SchemaIssue'

export interface Undecodable {
  readonly _tag: 'Undecodable'
  readonly id?: string
  readonly path: string
  readonly reason: string
}
export interface Unrecognized {
  readonly _tag: 'Unrecognized'
  readonly raw: unknown
}
export interface UnrecognizedLeaf {
  readonly path: string
  readonly value: Unrecognized
}

/** Open literal leaves, preserving branded identities, rich values and union discriminators. */
export type Open<T> = [T] extends [boolean] ? boolean extends T ? T : T | Unrecognized
  : T extends Brand.Brand<string> | DateTime.Utc | Duration.Duration ? T
  : T extends { readonly _tag: 'Unknown'; readonly raw: string } ? Unrecognized
  : T extends string ? string extends T ? T : T extends `${string}/${string}` ? T : T | Unrecognized
  : T extends number ? number extends T ? T : T | Unrecognized
  : T extends boolean ? boolean extends T ? T : T | Unrecognized
  : T extends Option.Option<infer TValue> ? Option.Option<Open<TValue>>
  : T extends readonly (infer TValue)[] ? readonly Open<TValue>[]
  : T extends object ? { readonly [TKey in keyof T]: TKey extends 'kind' | '_tag' | 'type' | 'op' ? T[TKey] : Open<T[TKey]> }
  : T

type DecodeMetadata = { readonly decode?: { readonly unrecognized: readonly UnrecognizedLeaf[] } }
export type DecodedAgent = Open<Agent> & DecodeMetadata
export type DecodedUsageSummary = Open<UsageSummary>
/** Only roster resources opt into open leaves; other resources remain generated-codec checked. */
export type DecodedResource = (Exclude<Resource, Agent> | Open<Agent>) & DecodeMetadata
export interface ResourceDecodeResult {
  readonly value?: DecodedResource
  readonly undecodable: readonly Undecodable[]
}

const object = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value)
const validators = new WeakMap<SchemaAST.AST, (value: unknown) => boolean>()
const accepts = (ast: SchemaAST.AST, value: unknown): boolean => {
  let validate = validators.get(ast)
  if (validate === undefined) {
    validate = Schema.is(Schema.make<Schema.Codec<unknown, unknown>>(ast))
    validators.set(ast, validate)
  }
  return validate(value)
}

const UnrecognizedSchema = Schema.Struct({ _tag: Schema.Literal('Unrecognized'), raw: Schema.Unknown })
const widened = new WeakMap<SchemaAST.AST, SchemaAST.AST>()

/** AST nodes carry non-public parser methods on their prototype. Preserve those methods
 * when replacing link/context fields not exposed by this Effect version's public API. */
const rebuildNode = (ast: SchemaAST.AST, fields: Partial<Pick<SchemaAST.AST, 'encoding' | 'context'>>): SchemaAST.AST =>
  Object.create(Object.getPrototypeOf(ast), Object.getOwnPropertyDescriptors({ ...ast, ...fields }))
/** Rebuild both sides of codec links, retaining generated rich transformations and checks.
 * Unknown primitive cases are real tagged values, never fabricated known literal values.
 */
const openLeaves = (ast: SchemaAST.AST): SchemaAST.AST => {
  const cached = widened.get(ast)
  if (cached !== undefined) return cached
  let result = ast
  if (ast._tag === 'Literal' || ast._tag === 'Enum'
    || ast._tag === 'Union' && ast.types.every((type) => type._tag === 'Literal')) {
    const unknown = Schema.Unknown.check(Schema.makeFilter((raw: unknown) =>
      (typeof raw === 'string' || typeof raw === 'number' || typeof raw === 'boolean')
        && (ast._tag === 'Literal' ? typeof raw === typeof ast.literal
          : ast._tag === 'Union' ? ast.types.some((type) => type._tag === 'Literal' && typeof raw === typeof type.literal)
          : typeof raw === 'string' || typeof raw === 'number')
        && !accepts(ast, raw),
    { expected: 'an unrecognized primitive literal' })).pipe(Schema.decodeTo(
      UnrecognizedSchema,
      SchemaTransformation.transform({
        decode: (raw: unknown): Unrecognized => ({ _tag: 'Unrecognized', raw }),
        encode: (value: Unrecognized) => value.raw,
      }),
    ))
    const known = ast.encoding === undefined ? ast : rebuildNode(ast, { encoding: undefined })
    const union = Schema.Union([Schema.make<Schema.Codec<unknown, unknown>>(known), unknown]).ast
    result = rebuildNode(ast.encoding === undefined ? union : SchemaAST.toType(union), { context: ast.context })
  } else if (ast._tag === 'Objects') {
    result = new SchemaAST.Objects(
      ast.propertySignatures.map((property) => new SchemaAST.PropertySignature(property.name,
        property.name === 'kind' || property.name === '_tag' || property.name === 'type' || property.name === 'op'
          ? property.type : ast.encoding === undefined ? openLeaves(property.type) : SchemaAST.toType(openLeaves(property.type)))),
      ast.indexSignatures.map((index) => new SchemaAST.IndexSignature(index.parameter, openLeaves(index.type))),
      ast.annotations, ast.checks, ast.encoding, ast.context, ast.encodingChecks,
    )
  } else if (ast._tag === 'Arrays') {
    result = new SchemaAST.Arrays(ast.isMutable,
      ast.elements.map((type) => ast.encoding === undefined ? openLeaves(type) : SchemaAST.toType(openLeaves(type))),
      ast.rest.map((type) => ast.encoding === undefined ? openLeaves(type) : SchemaAST.toType(openLeaves(type))),
      ast.annotations, ast.checks, ast.encoding, ast.context, ast.encodingChecks)
  } else if (ast._tag === 'Union') {
    result = new SchemaAST.Union(ast.types.map((type) => ast.encoding === undefined ? openLeaves(type) : SchemaAST.toType(openLeaves(type))), ast.options,
      ast.annotations, ast.checks, ast.encoding, ast.context, ast.encodingChecks)
  } else if (ast._tag === 'Declaration') {
    result = new SchemaAST.Declaration(ast.typeParameters.map((type) => SchemaAST.toType(openLeaves(type))), ast.run,
      ast.annotations, ast.checks, ast.encoding, ast.context, ast.encodingChecks, ast.encodingRun)
  }
  if (ast.encoding !== undefined) {
    const link = (value: SchemaAST.Link) => new SchemaAST.Link(openLeaves(value.to), value.transformation)
    result = rebuildNode(result, { encoding: [link(ast.encoding[0]), ...ast.encoding.slice(1).map(link)] })
  }
  widened.set(ast, result)
  return result
}

// The type is the recursive counterpart of the AST transformation above. No wire schema changes.
const AgentSchema = Schema.make<Schema.Codec<DecodedAgent, unknown>>(openLeaves(Agent.ast))
const UsageSchema = Schema.make<Schema.Codec<DecodedUsageSummary, unknown>>(openLeaves(UsageSummary.ast))
const decodeAgent = Schema.decodeUnknownSync(AgentSchema)
const decodeResource = decodeUnknownSync(Resource)
export const decodedUsageEquivalence: (left: DecodedUsageSummary, right: DecodedUsageSummary) => boolean =
  Schema.toEquivalence(UsageSchema)

const unknownLeaves = (value: unknown, path: string, found: UnrecognizedLeaf[]): void => {
  if (object(value)) {
    if (value._tag === 'Unrecognized') {
      found.push({ path, value: { _tag: 'Unrecognized', raw: value.raw } })
      return
    }
    if (value._tag === 'Some' && Object.hasOwn(value, 'value')) {
      unknownLeaves(value.value, path, found)
      return
    }
    for (const [key, child] of Object.entries(value)) unknownLeaves(child, path ? `${path}.${key}` : key, found)
  } else if (Array.isArray(value)) value.forEach((child, index) => unknownLeaves(child, `${path}.${index}`, found))
}

/** Convert an already decoded generated agent at fixture/SDK integration boundaries. */
export const openAgent = (agent: Agent): DecodedAgent => {
  const value = decodeAgent(Schema.encodeSync(Agent)(agent))
  const unrecognized: UnrecognizedLeaf[] = []
  unknownLeaves(value, '', unrecognized)
  return unrecognized.length === 0 ? value : { ...value, decode: { unrecognized } }
}

/** Identity and structure are closed; future agent leaf cases remain in their own row. */
export const decodeResourceRow = (raw: unknown): ResourceDecodeResult => {
  const id = object(raw) && typeof raw.id === 'string' ? raw.id : undefined
  const diagnostic = (path: string, reason: string): Undecodable => ({
    _tag: 'Undecodable', ...(id === undefined ? {} : { id }), path, reason,
  })
  try {
    if (!object(raw) || raw.kind !== 'agent') {
      const value = decodeResource(raw)
      if (value.kind === 'agent')
        return { undecodable: [diagnostic('kind', 'Resource identity did not select the agent codec')] }
      if (typeof value.kind !== 'string')
        return { undecodable: [diagnostic('kind', 'Unrecognized resource identity discriminator')] }
      return { value, undecodable: [] }
    }
    const value = decodeAgent(raw)
    if (typeof value.kind !== 'string')
      return { undecodable: [diagnostic('kind', 'Unrecognized resource identity discriminator')] }
    const unrecognized: UnrecognizedLeaf[] = []
    unknownLeaves(value, '', unrecognized)
    return {
      value: unrecognized.length > 0 && value.kind === 'agent' ? { ...value, decode: { unrecognized } } : value,
      undecodable: unrecognized.map((leaf) => diagnostic(leaf.path, 'Unrecognized leaf value')),
    }
  } catch (error) {
    const failures: Undecodable[] = []
    if (object(raw) && raw.kind === 'agent' && Schema.isSchemaError(error)) {
      const visit = (issue: Issue, path: readonly PropertyKey[]): void => {
        if (issue._tag === 'Pointer') visit(issue.issue, [...path, ...issue.path])
        else if ('issues' in issue && issue.issues.length > 0)
          issue.issues.forEach((child) => visit(child, path))
        else if ('issue' in issue) visit(issue.issue, path)
        else failures.push(diagnostic(path.length === 0 ? '$' : path.map(String).join('.'),
          'Value does not satisfy the resource field schema'))
      }
      visit(error.issue, [])
      if (failures.length > 0) return { undecodable: failures }
    }
    const encoded = SchemaAST.toEncoded(Resource.ast)
    const selected = encoded._tag === 'Union' && object(raw) ? encoded.types.find((type) =>
      type._tag === 'Objects' && type.propertySignatures.some((property) => property.name === 'kind'
        && property.type._tag === 'Literal' && property.type.literal === raw.kind)) : undefined
    if (selected?._tag === 'Objects' && object(raw)) {
      for (const property of selected.propertySignatures) {
        const key = String(property.name)
        if (!accepts(property.type, raw[key])) failures.push(diagnostic(key, 'Value does not satisfy the resource field schema'))
      }
    }
    return { undecodable: failures.length > 0 ? failures : [diagnostic('$', 'Value does not satisfy the resource schema')] }
  }
}

export const decodeResourceRows = (rows: readonly unknown[]): {
  readonly items: readonly DecodedResource[]
  readonly undecodable: readonly Undecodable[]
} => {
  const items: DecodedResource[] = []
  const undecodable: Undecodable[] = []
  for (const raw of rows) {
    const decoded = decodeResourceRow(raw)
    if (decoded.value !== undefined) items.push(decoded.value)
    undecodable.push(...decoded.undecodable)
  }
  return { items, undecodable }
}

/** One SDK session owns these counters and first-seen paths. No row contents are emitted. */
export const makeDecodeDiagnostics = (event: (resource: string, path: string, count: number, first: boolean) => void) => {
  const counts = new Map<string, number>()
  return (resource: string, diagnostics: readonly Undecodable[]): void => {
    for (const diagnostic of diagnostics) {
      const key = `${resource}:${diagnostic.path}`
      const count = (counts.get(key) ?? 0) + 1
      counts.set(key, count)
      event(resource, diagnostic.path, count, count === 1)
    }
  }
}
