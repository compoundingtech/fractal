let observer: IntersectionObserver | undefined
const targets = new WeakMap<Element, Set<HTMLElement | SVGElement>>()

/**
 * React 19 ref callback: one shared observer pauses animations while their fixed parent boxes are off screen.
 * Observing the rotating glyph itself makes Chromium recompute intersections every frame.
 * Without IntersectionObserver (server render, jsdom) the animation simply keeps running.
 */
export function pauseOffscreen(element: HTMLElement | SVGElement | null): (() => void) | undefined {
  if (element === null) return undefined
  const box = element.parentElement
  if (typeof IntersectionObserver === 'undefined' || box === null) return undefined
  observer ??= new IntersectionObserver(entries => {
    for (const entry of entries) for (const target of targets.get(entry.target) ?? []) target.style.animationPlayState = entry.isIntersecting ? 'running' : 'paused'
  })
  let glyphs = targets.get(box)
  if (glyphs === undefined) {
    glyphs = new Set()
    targets.set(box, glyphs)
    observer.observe(box)
  }
  glyphs.add(element)
  return () => {
    glyphs.delete(element)
    if (glyphs.size === 0) { observer!.unobserve(box); targets.delete(box) }
    element.style.removeProperty('animation-play-state')
  }
}
