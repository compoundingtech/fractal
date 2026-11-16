import * as React from 'react'
import * as Atom from 'effect/reactivity/Atom'
import { useAtomValue } from '@effect/atom-react'

export interface ForegroundSnapshot {
  readonly foreground: boolean
  readonly visibleMessageIds: readonly string[]
}
const hidden: ForegroundSnapshot = { foreground: false, visibleMessageIds: [] }

/** Browser events and observers drive this atom; no timers or polling. */
export const foregroundVisibility = (container: HTMLElement | null, active: boolean): Atom.Atom<ForegroundSnapshot> => Atom.make(get => {
  if (!active || container === null || typeof IntersectionObserver === 'undefined') return hidden
  const intersecting = new Set<Element>()
  const tracked = new Set<Element>()
  let current: ForegroundSnapshot = hidden
  const rows = () => [...container.querySelectorAll<HTMLElement>('[data-thread-message]')]
  const foreground = () => document.visibilityState === 'visible' && document.hasFocus()
    && container.isConnected && container.closest('[hidden], [inert], [aria-hidden="true"]') === null
    && ![...document.querySelectorAll('dialog[open], [role="dialog"], [role="alertdialog"]')].some(dialog =>
      !dialog.contains(container) && dialog.closest('[hidden], [aria-hidden="true"]') === null
      && getComputedStyle(dialog).display !== 'none' && getComputedStyle(dialog).visibility !== 'hidden')
  const publish = () => {
    if (!foreground()) { if (current.foreground) { current = hidden; get.setSelf(current) }; return }
    const bounds = viewportBounds(container)
    const visibleMessageIds = rows().flatMap(row => {
      if (!intersecting.has(row)) return []
      const rect = row.getBoundingClientRect()
      const left = Math.max(rect.left, bounds.left), right = Math.min(rect.right, bounds.right)
      const top = Math.max(rect.top, bounds.top), bottom = Math.min(rect.bottom, bounds.bottom)
      if (right <= left || bottom <= top) return []
      // Intersections do not account for covering content. Check the actually exposed point too.
      const exposed = document.elementFromPoint?.((left + right) / 2, (top + bottom) / 2)
      if (exposed != null && !row.contains(exposed)) return []
      const id = row.getAttribute('data-thread-message')
      return id === null ? [] : [id]
    })
    const ids = [...new Set(visibleMessageIds)].slice(-50)
    if (current.foreground && current.visibleMessageIds.length === ids.length && current.visibleMessageIds.every((id, index) => id === ids[index])) return
    current = { foreground: true, visibleMessageIds: ids }
    get.setSelf(current)
  }
  const observer = new IntersectionObserver(entries => {
    for (const entry of entries) {
      if (tracked.has(entry.target) && entry.isIntersecting && entry.intersectionRatio > 0) intersecting.add(entry.target)
      else intersecting.delete(entry.target)
    }
    publish()
  }, { threshold: 0.001 })
  const sync = () => {
    const current = new Set(rows())
    for (const row of tracked) if (!current.has(row)) { observer.unobserve(row); tracked.delete(row); intersecting.delete(row) }
    for (const row of current) if (!tracked.has(row)) { tracked.add(row); observer.observe(row) }
    publish()
  }
  const mutations = new MutationObserver(sync)
  mutations.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['hidden', 'inert', 'aria-hidden', 'open', 'style', 'class', 'data-thread-message'] })
  for (const type of ['focus', 'blur', 'resize']) window.addEventListener(type, publish)
  document.addEventListener('visibilitychange', publish)
  document.addEventListener('scroll', publish, true)
  get.addFinalizer(() => {
    observer.disconnect(); mutations.disconnect()
    for (const type of ['focus', 'blur', 'resize']) window.removeEventListener(type, publish)
    document.removeEventListener('visibilitychange', publish)
    document.removeEventListener('scroll', publish, true)
  })
  sync()
  return current
})

export const useForegroundVisibility = (active: boolean): ForegroundSnapshot & {
  readonly timeline: React.Dispatch<React.SetStateAction<HTMLDivElement | null>>
} => {
  const [container, timeline] = React.useState<HTMLDivElement | null>(null)
  const visibility = React.useMemo(() => foregroundVisibility(container, active), [container, active])
  return { ...useAtomValue(visibility), timeline }
}

const viewportBounds = (container: HTMLElement) => {
  const bounds = { top: 0, bottom: window.innerHeight, left: 0, right: window.innerWidth }
  const clips = (overflow: string) => overflow !== '' && overflow !== 'visible'
  for (let element = container.parentElement; element !== null; element = element.parentElement) {
    const style = getComputedStyle(element)
    if (!clips(style.overflowX) && !clips(style.overflowY)) continue
    const rect = element.getBoundingClientRect()
    if (clips(style.overflowX)) {
      bounds.left = Math.max(bounds.left, rect.left + element.clientLeft)
      bounds.right = Math.min(bounds.right, rect.left + element.clientLeft + element.clientWidth)
    }
    if (clips(style.overflowY)) {
      bounds.top = Math.max(bounds.top, rect.top + element.clientTop)
      bounds.bottom = Math.min(bounds.bottom, rect.top + element.clientTop + element.clientHeight)
    }
  }
  const heading = container.closest('[data-thread-panel]')?.querySelector('[data-thread-read-heading]')
  if (heading) bounds.top = Math.max(bounds.top, heading.getBoundingClientRect().bottom)
  return bounds
}
