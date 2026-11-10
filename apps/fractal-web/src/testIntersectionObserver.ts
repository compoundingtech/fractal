/**
 * jsdom has no layout, so nothing ever enters view on its own. This model records observed targets
 * and lets a test decide when one becomes wholly visible; real clipping by the popover and the
 * viewport is covered by story plays. Install it with `vi.stubGlobal('IntersectionObserver', …)`.
 */
export const createIntersectionObserverModel = () => {
  const live = new Set<Model>()
  class Model implements IntersectionObserver {
    readonly root = null
    readonly rootMargin = '0px'
    readonly scrollMargin = '0px'
    readonly thresholds: readonly number[]
    readonly targets = new Set<Element>()
    constructor(readonly callback: IntersectionObserverCallback, options?: IntersectionObserverInit) {
      const threshold = options?.threshold ?? 0
      this.thresholds = typeof threshold === 'number' ? [threshold] : threshold
    }
    observe(target: Element) { this.targets.add(target); live.add(this) }
    unobserve(target: Element) { this.targets.delete(target) }
    disconnect() { this.targets.clear(); live.delete(this) }
    takeRecords(): IntersectionObserverEntry[] { return [] }
  }
  return {
    IntersectionObserver: Model,
    /** Elements currently observed by any live observer. */
    observed: (): readonly Element[] => [...live].flatMap((observer) => [...observer.targets]),
    /** Reports `target` as wholly visible to every observer watching it. */
    reveal: (target: Element): void => {
      const rect = target.getBoundingClientRect()
      for (const observer of [...live]) {
        if (!observer.targets.has(target)) continue
        observer.callback([{ target, isIntersecting: true, intersectionRatio: 1, boundingClientRect: rect, intersectionRect: rect, rootBounds: null, time: 0 }], observer)
      }
    },
  }
}
