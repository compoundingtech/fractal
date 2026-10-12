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
