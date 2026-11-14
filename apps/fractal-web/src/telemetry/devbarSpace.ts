/** Fixed chrome's strip height, published so the workbench reserves it without subscribing to the bar's preference. */
const spaceProperty = '--wf-devbar-space'

/** Measures only the bar's row: a panel opening above it overlays the workspace and reserves nothing. */
export const reserveDevbarSpace = (host: HTMLElement): (() => void) => {
  const strip = host.querySelector<HTMLElement>('[role="toolbar"][aria-label="Developer bar"]')
  if (strip === null) return () => undefined
  const style = host.ownerDocument.body.style
  const measure = () => style.setProperty(spaceProperty, `${strip.getBoundingClientRect().height}px`)
  measure()
  const observer = typeof ResizeObserver === 'undefined' ? undefined : new ResizeObserver(measure)
  observer?.observe(strip)
  return () => {
    observer?.disconnect()
    style.removeProperty(spaceProperty)
  }
}
