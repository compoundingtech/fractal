import { isPane, sharesOf, type Layout } from './layout.ts'

export interface Rect {
  readonly x: number
  readonly y: number
  readonly width: number
  readonly height: number
}

export type Direction = 'left' | 'right' | 'up' | 'down'

/** Each pane's rectangle within `area` (the unit square by default), from the split sizes alone. */
export const paneRects = (layout: Layout, area: Rect = { x: 0, y: 0, width: 1, height: 1 }): ReadonlyMap<string, Rect> => {
  const rects = new Map<string, Rect>()
  const walk = (node: Layout, rect: Rect) => {
    if (isPane(node)) {
      rects.set(node.id, rect)
      return
    }
    const shares = sharesOf(node)
    let offset = 0
    node.children.forEach((child, index) => {
      const share = shares[index]!
      walk(child, node.axis === 'row'
        ? { x: rect.x + rect.width * offset, y: rect.y, width: rect.width * share, height: rect.height }
        : { x: rect.x, y: rect.y + rect.height * offset, width: rect.width, height: rect.height * share })
      offset += share
    })
  }
  walk(layout, area)
  return rects
}

const epsilon = 1e-9

/**
 * The pane nearest in a direction, by centre distance among the panes entirely on that side;
 * `undefined` at the edge of the layout, which a host may route elsewhere (say, a sidebar on the left).
 */
export const neighbour = (rects: ReadonlyMap<string, Rect>, from: string, direction: Direction): string | undefined => {
  const origin = rects.get(from)
  if (origin === undefined) return undefined
  const centre = (rect: Rect) => [rect.x + rect.width / 2, rect.y + rect.height / 2] as const
  const [fx, fy] = centre(origin)
  let best: { readonly id: string; readonly distance: number } | undefined
  for (const [id, rect] of rects) {
    if (id === from) continue
    const beyond = direction === 'left' ? rect.x + rect.width <= origin.x + epsilon
      : direction === 'right' ? rect.x >= origin.x + origin.width - epsilon
      : direction === 'up' ? rect.y + rect.height <= origin.y + epsilon
      : rect.y >= origin.y + origin.height - epsilon
    if (!beyond) continue
    const [x, y] = centre(rect)
    const distance = Math.abs(x - fx) + Math.abs(y - fy)
    if (best === undefined || distance < best.distance - epsilon) best = { id, distance }
  }
  return best?.id
}
