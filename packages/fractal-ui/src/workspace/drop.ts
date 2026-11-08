import type { Rect } from './geometry.ts'
import { findPane, locateTab, type Layout } from './layout.ts'
import type { Drop, Side } from './model.ts'

export type DropZone = { readonly _tag: 'Center' } | { readonly _tag: 'Edge'; readonly side: Side }

export interface Point {
  readonly x: number
  readonly y: number
}

/**
 * Which zone of a pane's content a point is in: within `edge` (a fraction of the pane's width or
 * height, or pixels when above 1) of a side is that side's edge, the nearest side winning; else the centre.
 */
export const resolveDropZone = (rect: Rect, point: Point, options?: { readonly edge?: number | undefined }): DropZone => {
  const edge = options?.edge ?? 0.25
  const bandX = edge > 1 ? edge : rect.width * edge
  const bandY = edge > 1 ? edge : rect.height * edge
  const distances: ReadonlyArray<readonly [Side, number, number]> = [
    ['left', point.x - rect.x, bandX],
    ['right', rect.x + rect.width - point.x, bandX],
    ['top', point.y - rect.y, bandY],
    ['bottom', rect.y + rect.height - point.y, bandY],
  ]
  let best: readonly [Side, number] | undefined
  for (const [side, distance, band] of distances) {
    if (distance > band) continue
    const relative = band === 0 ? 0 : distance / band
    if (best === undefined || relative < best[1]) best = [side, relative]
  }
  return best === undefined ? { _tag: 'Center' } : { _tag: 'Edge', side: best[0] }
}

/** The slot index in a horizontal strip: before the first tab whose midpoint is right of `x`. */
export const stripInsertIndex = (tabs: readonly Pick<Rect, 'x' | 'width'>[], x: number): number => {
  const index = tabs.findIndex(tab => x < tab.x + tab.width / 2)
  return index === -1 ? tabs.length : index
}

export const dropFor = (pane: string, zone: DropZone): Drop =>
  zone._tag === 'Center' ? { _tag: 'Center', pane } : { _tag: 'Edge', pane, side: zone.side }

/**
 * Whether dropping a tab here would change nothing: its own single-tab pane's edge or centre, or its
 * own slot. A ref not in the layout always lands.
 */
export const isNoopDrop = (layout: Layout, tab: string | undefined, drop: Drop): boolean => {
  if (tab === undefined) return false
  const from = locateTab(layout, row => row.id === tab)
  if (from === undefined || findPane(layout, drop.pane) === undefined) return false
  if (from.pane.id !== drop.pane) return false
  if (drop._tag === 'Center') return true
  if (drop._tag === 'Edge') return from.pane.tabs.length === 1
  return from.pane.tabs.findIndex(row => row.id === tab) === Math.max(0, Math.min(from.pane.tabs.length - 1, Math.trunc(drop.index)))
}

/**
 * A strip slot (0 = before the first tab) as a `Strip` drop's index, which is where the tab ends up:
 * a tab moving right within its own strip vacates a slot on its way.
 */
export const stripDropIndex = (slot: number, fromIndex: number | undefined): number =>
  fromIndex !== undefined && fromIndex >= 0 && slot > fromIndex ? slot - 1 : slot
