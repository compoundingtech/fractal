let observer: IntersectionObserver | undefined

/**
 * React 19 ref callback for an element that runs an infinite CSS animation: one shared IntersectionObserver
 * pauses the element's animation while it is off screen, so only visible spinners tick.
 * Without IntersectionObserver (server render, jsdom) the animation simply keeps running.
 */
export function pauseOffscreen(element: HTMLElement | SVGElement): (() => void) | undefined {
  if (typeof IntersectionObserver === 'undefined') return undefined
  observer ??= new IntersectionObserver(entries => {
    for (const entry of entries) (entry.target as HTMLElement | SVGElement).style.animationPlayState = entry.isIntersecting ? 'running' : 'paused'
  })
  observer.observe(element)
  return () => {
    observer!.unobserve(element)
    element.style.removeProperty('animation-play-state')
  }
}
