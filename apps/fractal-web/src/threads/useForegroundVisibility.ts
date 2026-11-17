import * as React from 'react'
import * as Atom from 'effect/reactivity/Atom'
import { useAtomValue } from '@effect/atom-react'
import { meaningfulBodyRegion, overlap, type ReadableRect } from './meaningfulGeometry.ts'
import { threadAnchorAttribute, threadAnchorSelector, threadReadEligibleAttribute } from './threadAnchor.ts'

export interface ForegroundSnapshot {
  readonly foreground: boolean
  readonly visibleMessageIds: readonly string[]
}
const hidden: ForegroundSnapshot = { foreground: false, visibleMessageIds: [] }

// Adopted source: doc/threads-delivery/adopted-ux-20261116/unread-implementation,
// “Deterministic visibility default for implementation/review”: stable 500 ms,
// one cancellable placement timer, followed by a fresh body measurement.
const dwellMs = 500

interface VisibleCandidate {
  readonly id: string
  readonly body: HTMLElement
  readonly content: string
  readonly geometry: string
}

export const foregroundVisibility = (container: HTMLElement | null, active: boolean): Atom.Atom<ForegroundSnapshot> => Atom.make(get => {
  if (!active || container === null) return hidden
  let current: ForegroundSnapshot = hidden
  let timer: number | undefined
  const tracked = new Set<Element>()
  let measured: readonly VisibleCandidate[] = []
  let measuredForeground = false
  const foreground = () => document.visibilityState === 'visible' && document.hasFocus()
    && container.isConnected && shown(container)
    && ![...document.querySelectorAll('dialog[open], [role="dialog"], [role="alertdialog"]')].some(dialog =>
      !dialog.contains(container) && shown(dialog))
  const publish = (next: ForegroundSnapshot) => {
    if (current.foreground === next.foreground && current.visibleMessageIds.length === next.visibleMessageIds.length
      && current.visibleMessageIds.every((id, index) => id === next.visibleMessageIds[index])) return
    current = next
    get.setSelf(current)
  }
  const measure = () => {
    const candidates: VisibleCandidate[] = []
    if (!foreground()) return candidates
    for (const row of container.querySelectorAll<HTMLElement>(threadAnchorSelector)) {
      if (!row.hasAttribute(threadReadEligibleAttribute)) continue
      const body = row.querySelector<HTMLElement>('[data-thread-message-content]')
      const id = row.getAttribute(threadAnchorAttribute)
      if (body === null || id === null || !body.isConnected || !body.textContent?.trim() || !shown(body)) continue
      const rect = body.getBoundingClientRect()
      const style = getComputedStyle(body)
      const lineHeight = Number.parseFloat(style.lineHeight)
      const clips = viewportClips(body, container)
      const region = meaningfulBodyRegion(rect, Number.isFinite(lineHeight) ? lineHeight : renderedLineHeight(body), clips)
      if (region === undefined || !exposed(body, region)) continue
      candidates.push({ id, body, content: body.innerHTML, geometry: [rect.left, rect.right, rect.top, rect.bottom, region.top,
        ...clips.flatMap(clip => [clip.left, clip.right, clip.top, clip.bottom])].join(',') })
    }
    return candidates
  }
  const sameCandidates = (before: readonly VisibleCandidate[], after: readonly VisibleCandidate[]) =>
    before.length === after.length && before.every((candidate, index) => {
      const next = after[index]
      return next !== undefined && candidate.id === next.id && candidate.body === next.body
        && candidate.content === next.content && candidate.geometry === next.geometry
    })
  const cancel = () => { window.clearTimeout(timer); timer = undefined }
  const restart = () => {
    cancel()
    measuredForeground = foreground()
    publish(measuredForeground ? { foreground: true, visibleMessageIds: [] } : hidden)
    const before = measure()
    measured = before
    if (before.length === 0) return
    timer = window.setTimeout(() => {
      timer = undefined
      const after = measure()
      if (!sameCandidates(before, after)) { restart(); return }
      publish(foreground() ? { foreground: true, visibleMessageIds: [...new Set(after.map(candidate => candidate.id))].slice(-50) } : hidden)
    }, dwellMs)
  }
  // Global overlay discovery must not require silence in unrelated streaming panes.
  const refresh = () => {
    if (foreground() !== measuredForeground || !sameCandidates(measured, measure())) restart()
  }
  const intersections = typeof IntersectionObserver === 'undefined' ? undefined : new IntersectionObserver(refresh, { threshold: [0, 1] })
  const resize = typeof ResizeObserver === 'undefined' ? undefined : new ResizeObserver(refresh)
  const sync = () => {
    const elements = new Set<Element>([container, ...container.querySelectorAll(`${threadAnchorSelector}, [data-thread-message-content]`)])
    for (let ancestor = container.parentElement; ancestor !== null; ancestor = ancestor.parentElement) elements.add(ancestor)
    const panel = container.closest('[data-thread-panel]')
    for (const element of panel?.querySelectorAll('[data-thread-read-heading], [data-thread-composer], [data-testid="conversation-composer-dock"], textarea, form') ?? []) elements.add(element)
    for (const element of tracked) if (!elements.has(element)) { resize?.unobserve(element); intersections?.unobserve(element); tracked.delete(element) }
    for (const element of elements) if (!tracked.has(element)) { tracked.add(element); resize?.observe(element); intersections?.observe(element) }
    refresh()
  }
  const mutations = new MutationObserver(sync)
  mutations.observe(document.body, { subtree: true, childList: true, characterData: true, attributes: true,
    attributeFilter: ['hidden', 'inert', 'aria-hidden', 'open', 'style', 'class', threadAnchorAttribute, threadReadEligibleAttribute, 'data-thread-message-content'] })
  for (const type of ['focus', 'blur', 'resize']) window.addEventListener(type, restart)
  document.addEventListener('visibilitychange', restart)
  document.addEventListener('scroll', restart, true)
  window.visualViewport?.addEventListener('resize', restart)
  window.visualViewport?.addEventListener('scroll', restart)
  get.addFinalizer(() => {
    cancel(); intersections?.disconnect(); resize?.disconnect(); mutations.disconnect()
    for (const type of ['focus', 'blur', 'resize']) window.removeEventListener(type, restart)
    document.removeEventListener('visibilitychange', restart)
    document.removeEventListener('scroll', restart, true)
    window.visualViewport?.removeEventListener('resize', restart)
    window.visualViewport?.removeEventListener('scroll', restart)
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

const shown = (element: Element): boolean => {
  for (let ancestor: Element | null = element; ancestor !== null; ancestor = ancestor.parentElement) {
    if (ancestor.matches('[hidden], [inert], [aria-hidden="true"]')) return false
    const style = getComputedStyle(ancestor)
    if (style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse' || style.opacity === '0' || style.contentVisibility === 'hidden') return false
  }
  return true
}

const renderedLineHeight = (body: HTMLElement): number => {
  // With `normal`, line box origins are more accurate than font glyph bounds.
  const range = document.createRange()
  range.selectNodeContents(body)
  const lines = typeof range.getClientRects === 'function' ? [...range.getClientRects()] : []
  const tops = [...new Set(lines.map(line => line.top))].sort((a, b) => a - b)
  const first = tops[0], second = tops[1]
  return first !== undefined && second !== undefined ? second - first : body.getBoundingClientRect().height
}

const viewportClips = (body: HTMLElement, container: HTMLElement): ReadableRect[] => {
  const viewport = window.visualViewport
  const clips: ReadableRect[] = [{ left: 0, top: 0, right: window.innerWidth, bottom: window.innerHeight }]
  if (viewport) clips.push({ left: viewport.offsetLeft, top: viewport.offsetTop, right: viewport.offsetLeft + viewport.width, bottom: viewport.offsetTop + viewport.height })
  const clipsAxis = (overflow: string) => overflow !== '' && overflow !== 'visible'
  for (let element = body.parentElement; element !== null; element = element.parentElement) {
    const style = getComputedStyle(element), rect = element.getBoundingClientRect()
    clips.push({ left: clipsAxis(style.overflowX || style.overflow) ? rect.left + element.clientLeft : -Infinity,
      right: clipsAxis(style.overflowX || style.overflow) ? rect.left + element.clientLeft + element.clientWidth : Infinity,
      top: clipsAxis(style.overflowY || style.overflow) ? rect.top + element.clientTop : -Infinity,
      bottom: clipsAxis(style.overflowY || style.overflow) ? rect.top + element.clientTop + element.clientHeight : Infinity })
  }
  const heading = container.closest('[data-thread-panel]')?.querySelector('[data-thread-read-heading]')
  if (heading) clips.push({ left: -Infinity, right: Infinity, top: heading.getBoundingClientRect().bottom, bottom: Infinity })
  return clips
}

/** Probe the entire required region's boundary and every overlapping element's
 * intersection, not only the body midpoint (which misses partial composer covers).
 * Hit testing resolves paint order, so underlying siblings are not occluders.
 */
const exposed = (body: HTMLElement, region: ReadableRect): boolean => {
  if (typeof document.elementFromPoint !== 'function') return false
  // Composer docks can opt out of pointer hit testing while still covering text.
  for (const composer of document.querySelectorAll('[data-thread-composer], [data-testid="conversation-composer-dock"], [data-thread-panel] textarea')) {
    if (shown(composer) && overlap(region, composer.getBoundingClientRect()) !== undefined) return false
  }
  const clearAt = (x: number, y: number) => {
    const hit = document.elementFromPoint(x, y)
    return hit !== null && body.contains(hit)
  }
  const inset = Math.min(0.5, (region.right - region.left) / 4, (region.bottom - region.top) / 4)
  for (const x of [region.left + inset, (region.left + region.right) / 2, region.right - inset]) {
    for (const y of [region.top + inset, (region.top + region.bottom) / 2, region.bottom - inset]) if (!clearAt(x, y)) return false
  }
  for (const element of document.querySelectorAll('*')) {
    if (element.contains(body) || body.contains(element)) continue
    for (const rect of element.getClientRects()) {
      const intersection = overlap(region, rect)
      if (intersection !== undefined && !clearAt((intersection.left + intersection.right) / 2, (intersection.top + intersection.bottom) / 2)) return false
    }
  }
  return true
}
