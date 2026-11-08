import { Schema } from 'effect'

/**
 * The layout tree: splits whose leaves are panes of ordered tabs. Ids of tabs, panes and splits are
 * local to one client and never part of the shared body; a tab's `ref` is the host's opaque entity key.
 */
export const Axis = Schema.Literals(['row', 'column'])
/** `row` lays children out left to right, `column` top to bottom. */
export type Axis = typeof Axis.Type

export const Tab = Schema.Struct({
  id: Schema.String,
  ref: Schema.String,
  title: Schema.optional(Schema.String),
})
export type Tab = typeof Tab.Type

export const Pane = Schema.Struct({
  id: Schema.String,
  tabs: Schema.Array(Tab),
  /** The tab this pane shows. Local to the client. */
  active: Schema.NullOr(Schema.String),
})
export type Pane = typeof Pane.Type

export interface Split {
  readonly id: string
  readonly axis: Axis
  readonly children: readonly Layout[]
  /** Each child's share, summing to 1. Equal shares when absent. */
  readonly sizes?: readonly number[] | undefined
}
export type Layout = Pane | Split

export const Layout: Schema.Codec<Layout> = Schema.Union([
  Pane,
  Schema.Struct({
    id: Schema.String,
    axis: Axis,
    children: Schema.Array(Schema.suspend((): Schema.Codec<Layout> => Layout)),
    sizes: Schema.optional(Schema.Array(Schema.Finite)),
  }),
])

export const isPane = (layout: Layout): layout is Pane => 'tabs' in layout

/** Every pane, left to right and top to bottom. */
export const panesOf = (layout: Layout): readonly Pane[] => isPane(layout) ? [layout] : layout.children.flatMap(panesOf)

export const splitsOf = (layout: Layout): readonly Split[] =>
  isPane(layout) ? [] : [layout, ...layout.children.flatMap(splitsOf)]

export const tabsOf = (layout: Layout): readonly Tab[] => panesOf(layout).flatMap(pane => pane.tabs)

export const findPane = (layout: Layout, id: string): Pane | undefined => panesOf(layout).find(pane => pane.id === id)

export const locateTab = (layout: Layout, predicate: (tab: Tab) => boolean): { readonly pane: Pane; readonly tab: Tab } | undefined => {
  for (const pane of panesOf(layout)) {
    const tab = pane.tabs.find(predicate)
    if (tab !== undefined) return { pane, tab }
  }
  return undefined
}

/** One tab per pane is on screen. */
export const visibleTabIds = (layout: Layout): ReadonlySet<string> =>
  new Set(panesOf(layout).flatMap(pane => pane.active === null ? [] : [pane.active]))

export const minimumSize = 0.05

/** Shares that sum to 1 with every child at least `minimumSize`; `undefined` for equal shares. */
export const normalizeSizes = (sizes: readonly number[] | undefined, count: number): readonly number[] | undefined => {
  if (sizes === undefined || sizes.length !== count || count < 2) return undefined
  if (!sizes.every(size => Number.isFinite(size) && size > 0)) return undefined
  const total = sizes.reduce((sum, size) => sum + size, 0)
  const clamped = sizes.map(size => Math.max(minimumSize, size / total))
  const sum = clamped.reduce((acc, size) => acc + size, 0)
  return clamped.map(size => size / sum)
}

/** Each child's share, equal when the split has no sizes. */
export const sharesOf = (split: Split): readonly number[] =>
  normalizeSizes(split.sizes, split.children.length) ?? split.children.map(() => 1 / split.children.length)

export const mapPane = (layout: Layout, id: string, replace: (pane: Pane) => Layout): Layout => {
  if (isPane(layout)) return layout.id === id ? replace(layout) : layout
  const children = layout.children.map(child => mapPane(child, id, replace))
  return children.every((child, index) => child === layout.children[index]) ? layout : { ...layout, children }
}

export const mapSplit = (layout: Layout, id: string, replace: (split: Split) => Split): Layout => {
  if (isPane(layout)) return layout
  if (layout.id === id) return replace(layout)
  const children = layout.children.map(child => mapSplit(child, id, replace))
  return children.every((child, index) => child === layout.children[index]) ? layout : { ...layout, children }
}

const withSizes = (split: Omit<Split, 'sizes'>, sizes: readonly number[] | undefined): Split =>
  sizes === undefined ? split : { ...split, sizes }

/**
 * The layout without a pane; `undefined` when that was its only pane. A split left with one child
 * becomes that child, and the remaining children keep their relative shares.
 */
export const removePane = (layout: Layout, id: string): Layout | undefined => {
  if (isPane(layout)) return layout.id === id ? undefined : layout
  const shares = layout.sizes === undefined ? undefined : sharesOf(layout)
  const kept: Layout[] = []
  const keptShares: number[] = []
  layout.children.forEach((child, index) => {
    const next = removePane(child, id)
    if (next === undefined) return
    kept.push(next)
    if (shares !== undefined) keptShares.push(shares[index]!)
  })
  if (kept.length === 0) return undefined
  if (kept.length === 1) return kept[0]
  if (kept.length === layout.children.length && kept.every((child, index) => child === layout.children[index])) return layout
  const { sizes: _sizes, ...rest } = layout
  return withSizes({ ...rest, children: kept }, shares === undefined ? undefined : normalizeSizes(keptShares, kept.length))
}
