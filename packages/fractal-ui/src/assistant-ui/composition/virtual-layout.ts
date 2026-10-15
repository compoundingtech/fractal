import type { Key, ListLayout } from 'react-aria-components'

/** Shared RAC configuration. Estimates bootstrap unmounted rows; observed sizes own geometry. */
export const measuredListLayoutOptions = { estimatedRowHeight: 160 } as const

/** Reveal a collection key, leaving room for a sticky inset. Returns false until RAC has its key. */
export function scrollToLayoutKey(layout: ListLayout<unknown>, key: Key, viewport: HTMLElement, topInset = 0): boolean {
  const info = layout.getLayoutInfo(key)
  if (info === null) return false
  viewport.scrollTop = info.rect.y - topInset
  return true
}

/** Resolve scroll geometry even before RAC mounts the newly visible row. */
export function layoutKeyAtOffset(layout: ListLayout<unknown>, rows: readonly { readonly id: Key }[], offset: number) {
  let low = 0
  let high = rows.length - 1
  while (low <= high) {
    const middle = (low + high) >>> 1
    const key = rows[middle]!.id
    const info = layout.getLayoutInfo(key)
    if (info === null) return undefined
    if (offset < info.rect.y) high = middle - 1
    else if (offset >= info.rect.y + info.rect.height) low = middle + 1
    else return { key, info }
  }
  return undefined
}
