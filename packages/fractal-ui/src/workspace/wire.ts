import { Schema } from 'effect'
import { Axis, isPane, normalizeSizes, panesOf, splitsOf, type Layout, type Pane, type Split, type Tab } from './layout.ts'

/**
 * A space's shared body: its name and its structure, without any client-local identity. This is what
 * other windows and external layout stores exchange, as whole bodies.
 */
export const WireTab = Schema.Struct({ ref: Schema.String, title: Schema.optional(Schema.String) })
export type WireTab = typeof WireTab.Type

export type WireLayout =
  | { readonly tabs: readonly WireTab[] }
  | { readonly axis: Axis; readonly children: readonly WireLayout[]; readonly sizes?: readonly number[] | undefined }

export const WireLayout: Schema.Codec<WireLayout> = Schema.Union([
  Schema.Struct({ tabs: Schema.Array(WireTab) }),
  Schema.Struct({
    axis: Axis,
    children: Schema.Array(Schema.suspend((): Schema.Codec<WireLayout> => WireLayout)),
    sizes: Schema.optional(Schema.Array(Schema.Finite)),
  }),
])

export const SpaceBody = Schema.Struct({ name: Schema.String, layout: WireLayout })
export type SpaceBody = typeof SpaceBody.Type

const wireTab = ({ ref, title }: Tab): WireTab => title === undefined ? { ref } : { ref, title }

/** A layout's structure on the wire; sizes only when asked for. */
export const toWire = (layout: Layout, options: { readonly sizes: boolean }): WireLayout => {
  if (isPane(layout)) return { tabs: layout.tabs.map(wireTab) }
  const children = layout.children.map(child => toWire(child, options))
  const sizes = options.sizes ? normalizeSizes(layout.sizes, layout.children.length) : undefined
  return sizes === undefined ? { axis: layout.axis, children } : { axis: layout.axis, children, sizes }
}

/** Stable JSON: key order fixed, so two equal bodies serialise to the same string. */
export const canonicalBody = (body: SpaceBody): string => JSON.stringify(canonical(body))

const canonical = (value: unknown): unknown => {
  if (Array.isArray(value)) return value.map(canonical)
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().flatMap(key => {
      const entry = (value as Record<string, unknown>)[key]
      return entry === undefined ? [] : [[key, canonical(entry)]]
    }))
  }
  return value
}

/**
 * A binary rendering for stores whose splits are two-way: longer splits nest two at a time, the first
 * child against the rest, with the first child's share carried as a two-way size.
 */
export const toBinary = (layout: WireLayout): WireLayout => {
  if ('tabs' in layout) return layout
  const [first, ...rest] = layout.children
  if (first === undefined) return { tabs: [] }
  if (rest.length === 0) return toBinary(first)
  const shares = normalizeSizes(layout.sizes, layout.children.length)
  const restShares = shares?.slice(1)
  const restTotal = restShares?.reduce((sum, size) => sum + size, 0)
  const second: WireLayout = rest.length === 1
    ? rest[0]!
    : restShares === undefined || restTotal === undefined
      ? { axis: layout.axis, children: rest }
      : { axis: layout.axis, children: rest, sizes: restShares.map(size => size / restTotal) }
  const children = [toBinary(first), toBinary(second)] as const
  return shares === undefined ? { axis: layout.axis, children } : { axis: layout.axis, children, sizes: [shares[0]!, 1 - shares[0]!] }
}

export interface AdoptContext {
  readonly identity: (ref: string) => string
  readonly newId: () => string
}

/**
 * Takes a received structure wholesale while keeping this client's identities where they still apply:
 * tab and pane ids by entity identity, each pane's shown tab, split ids by position and axis, and
 * (unless `sizes`) local sizes. Empty panes stay exactly as received. A second tab for one identity, an
 * empty ref, a childless split and a one-child split do not survive. Unknown refs always do.
 */
export const adoptLayout = (current: Layout, received: WireLayout, context: AdoptContext & { readonly sizes: boolean }): Layout => {
  const localPanes = panesOf(current)
  const localTabs = new Map(localPanes.flatMap(pane => pane.tabs.map(tab => [context.identity(tab.ref), { tab, pane }] as const)).toReversed())
  const localSplits = splitsOf(current)
  const seen = new Set<string>(), paneIds = new Set<string>(), tabIds = new Set<string>(), splitIds = new Set<string>()
  let paneIndex = 0
  // Local panes a received tab claims by identity keep that claim over any positional match.
  const claimed = new Set<string>()
  const claim = (node: WireLayout): void => {
    if ('tabs' in node) {
      for (const tab of node.tabs) {
        const pane = localTabs.get(context.identity(tab.ref))?.pane.id
        if (pane !== undefined) claimed.add(pane)
      }
    } else node.children.forEach(claim)
  }
  claim(received)
  const unused = (used: Set<string>, ...candidates: readonly (string | undefined)[]) => {
    const id = candidates.find(candidate => candidate !== undefined && !used.has(candidate)) ?? context.newId()
    used.add(id)
    return id
  }
  // A split keeps its id when a local split of the same axis led with the same pane.
  const splitFor = (axis: Axis, firstPane: string | undefined): Split | undefined =>
    firstPane === undefined ? undefined : localSplits.find(split => split.axis === axis && panesOf(split)[0]?.id === firstPane && !splitIds.has(split.id))
  const walk = (node: WireLayout): Layout | undefined => {
    if (!('tabs' in node)) {
      const kept: Layout[] = []
      const keptSizes: number[] = []
      const shares = normalizeSizes(node.sizes, node.children.length)
      node.children.forEach((child, index) => {
        const next = walk(child)
        if (next === undefined) return
        kept.push(next)
        if (shares !== undefined) keptSizes.push(shares[index]!)
      })
      if (kept.length < 2) return kept[0]
      const local = splitFor(node.axis, panesOf(kept[0]!)[0]?.id)
      const id = unused(splitIds, local?.id)
      const sizes = context.sizes
        ? (shares === undefined ? undefined : normalizeSizes(keptSizes, kept.length))
        : (local !== undefined && local.children.length === kept.length ? normalizeSizes(local.sizes, kept.length) : undefined)
      return sizes === undefined ? { id, axis: node.axis, children: kept } : { id, axis: node.axis, children: kept, sizes }
    }
    const rows = node.tabs.filter(tab => {
      const identity = context.identity(tab.ref)
      if (tab.ref === '' || seen.has(identity)) return false
      seen.add(identity)
      return true
    })
    // Failing a tab it holds, a pane keeps the id of the local pane in its place.
    const local = localPanes[paneIndex++]?.id
    const position = local === undefined || claimed.has(local) ? undefined : local
    if (rows.length === 0) {
      const id = unused(paneIds, position)
      return { id, tabs: [], active: null }
    }
    const id = unused(paneIds, ...rows.map(tab => localTabs.get(context.identity(tab.ref))?.pane.id), position)
    const tabs = rows.map((row): Tab => {
      const tabId = unused(tabIds, localTabs.get(context.identity(row.ref))?.tab.id)
      return row.title === undefined ? { id: tabId, ref: row.ref } : { id: tabId, ref: row.ref, title: row.title }
    })
    const previous = localPanes.find(pane => pane.id === id)
    const shown = tabs.find(tab => tab.id === previous?.active)?.id
    const at = previous?.tabs.findIndex(tab => tab.id === previous.active) ?? 0
    const pane: Pane = { id, tabs, active: shown ?? tabs[Math.min(Math.max(at, 0), tabs.length - 1)]!.id }
    return pane
  }
  return walk(received) ?? { id: unused(paneIds, localPanes[0]?.id), tabs: [], active: null }
}
