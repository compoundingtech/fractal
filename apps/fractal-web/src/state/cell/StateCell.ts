import { Effect, Layer, ManagedRuntime, Schema } from 'effect'
import * as Atom from 'effect/reactivity/Atom'

import { defaultEnvironment } from './defaultEnvironment.ts'
import { CellEnvironment, layerMemory } from './environment.ts'
import * as Store from './store.ts'
import type { BootFields, CellBinding, CellStoreShape, Tier } from './store.ts'

export type { BootFields, CellIssue, Tier } from './store.ts'
export { bootKey } from './store.ts'

/**
 * Whose state this is. Every non-global key carries its owner, so glass sync can later replicate
 * a person's or a gateway's cells without a storage migration.
 * - `global`: one value per browser profile (panel layout).
 * - `gateway`: per connected gateway (sidebar lists).
 * - `person`: per signed-in person; `clearScope('person', id)` removes it on revoke and sign-out.
 * - `tab`: per workspace tab or subject (folds, disclosures).
 */
export type Scope = 'global' | 'gateway' | 'person' | 'tab'

interface SpecBase<A, I> {
  /** Stable name; never reuse a key for a different meaning. Bump `version` and add `migrate` instead. */
  readonly key: string
  readonly schema: Schema.Codec<A, I>
  readonly version: number
  readonly migrate?: (value: unknown, fromVersion: number) => unknown
  readonly default: A
  readonly tier?: Tier
  /** Required for `tier: 'boot'`: the flat fields the inline script paints from. */
  readonly boot?: (value: A) => BootFields
}
export interface GlobalSpec<A, I> extends SpecBase<A, I> {
  readonly scope: 'global'
  /** Reads a retired storage format once when nothing is stored yet. */
  readonly seed?: () => A | undefined
}
export interface ScopedSpec<A, I> extends SpecBase<A, I> {
  readonly scope: Exclude<Scope, 'global'>
  /** Bound the members of this cell per owner: least recently written go first, also under quota. */
  readonly retain?: number
  /** Reads a retired storage format once when nothing is stored yet for this owner and member. */
  readonly seed?: (owner: string, member?: string) => A | undefined
}

/** A scoped cell is a family: one owner id, plus an optional member (e.g. conversation) within it. */
export type CellFamily<A> = (owner: string, member?: string) => Atom.Writable<A>

const scopeTag: Record<Exclude<Scope, 'global'>, string> = { gateway: 'gw', person: 'p', tab: 't' }
const segment = (value: string) => encodeURIComponent(value)
/** `p:<person>/`: everything a person owns, across cells. */
export const scopePrefix = (scope: Exclude<Scope, 'global'>, owner: string) => `${scopeTag[scope]}:${segment(owner)}/`

const makeRuntime = () => ManagedRuntime.make(Store.layer().pipe(
  Layer.provide(defaultEnvironment()),
))
let runtime: ManagedRuntime.ManagedRuntime<Store.CellStore, never> | undefined
let installed: CellStoreShape | undefined
const store = (): CellStoreShape => {
  if (installed !== undefined) return installed
  runtime ??= makeRuntime()
  installed = runtime.runSync(Effect.service(Store.CellStore))
  return installed
}

/**
 * Replaces the store every cell resolves on its next read: tests install a memory store, embeds
 * install one so they never touch the host app's storage. Returns the previous store's disposer.
 */
export const install = (layer: Layer.Layer<Store.CellStore>): (() => Promise<void>) => {
  const previous = runtime
  runtime = ManagedRuntime.make(layer)
  installed = runtime.runSync(Effect.service(Store.CellStore))
  return () => previous?.dispose() ?? Promise.resolve()
}
/** A store over any environment, for tests: `layer(environment)`. */
export const layer = (environment: Layer.Layer<CellEnvironment>, options?: { readonly debounceMs?: number }) =>
  Store.layer(options).pipe(Layer.provide(environment))
export const memoryLayer = layer(layerMemory)

const atomOf = <A>(binding: () => CellBinding<A>): Atom.Writable<A> =>
  Atom.writable(
    (get) => {
      const cell = binding()
      const value = cell.read()
      get.addFinalizer(cell.subscribe(() => get.setSelf(cell.read())))
      return value
    },
    (ctx, value: A) => {
      binding().write(value)
      ctx.setSelf(value)
    },
  ).pipe(Atom.keepAlive)

/** A persisted UI state cell, readable on the first frame and shared across windows unless `tier: 'window'`. */
export function make<A, I>(spec: GlobalSpec<A, I>): Atom.Writable<A>
export function make<A, I>(spec: ScopedSpec<A, I>): CellFamily<A>
export function make<A, I>(spec: GlobalSpec<A, I> | ScopedSpec<A, I>): Atom.Writable<A> | CellFamily<A> {
  const tier = spec.tier ?? 'shared'
  if (tier === 'boot' && spec.boot === undefined) throw new Error(`Boot cell ${spec.key} needs a boot projection`)
  if (tier === 'sensitive' && spec.scope !== 'person') throw new Error(`Sensitive cell ${spec.key} must be person-scoped`)
  const bindingSpec = (key: string, seed: (() => A | undefined) | undefined, retain?: { readonly prefix: string; readonly max: number }): Store.CellBindingSpec<A, I> => ({
    key, tier, retain, seed,
    schema: spec.schema, version: spec.version, migrate: spec.migrate, default: spec.default, boot: spec.boot,
  })
  if (spec.scope === 'global') {
    const key = spec.key
    return atomOf(() => store().bind(bindingSpec(key, spec.seed)))
  }
  const family = new Map<string, Atom.Writable<A>>()
  const { scope, retain, seed } = spec
  return (owner, member) => {
    const familyPrefix = `${scopePrefix(scope, owner)}${spec.key}`
    const key = member === undefined ? familyPrefix : `${familyPrefix}/${segment(member)}`
    const existing = family.get(key)
    if (existing !== undefined) return existing
    const ownerSeed = seed === undefined ? undefined : () => seed(owner, member)
    const atom = atomOf(() => store().bind(bindingSpec(key, ownerSeed, retain === undefined ? undefined : { prefix: `${familyPrefix}/`, max: retain })))
    family.set(key, atom)
    return atom
  }
}

/** Revoke and sign-out: removes everything the owner has stored, in this and every other window. */
export const clearScope = (scope: Exclude<Scope, 'global'>, owner: string) => store().clearPrefix(scopePrefix(scope, owner))
export const flush = () => store().flush()
export const issues = () => store().issues()
export const ready = Effect.sync(() => { store() })
