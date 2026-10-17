/** All prompt geometry is read before publishing any overflow state. */
const measurements = new Map<HTMLElement, (overflows: boolean) => void>()
const pending = new Set<HTMLElement>()
let observer: ResizeObserver | undefined
let frame: number | undefined

function schedule(elements: readonly Element[]) {
  for (const element of elements) if (measurements.has(element as HTMLElement)) pending.add(element as HTMLElement)
  if (frame !== undefined || pending.size === 0) return
  frame = requestAnimationFrame(() => {
    frame = undefined
    const results = [...pending].flatMap(element => {
      const publish = measurements.get(element)
      return publish === undefined ? [] : [{ publish, overflows: element.scrollHeight > element.clientHeight }]
    })
    pending.clear()
    for (const { publish, overflows } of results) publish(overflows)
  })
}

/** Observe mount, text changes and resize without a synchronous per-row layout read. */
export function observeUserMessageOverflow(element: HTMLElement, publish: (overflows: boolean) => void): () => void {
  observer ??= new ResizeObserver(entries => schedule(entries.map(entry => entry.target)))
  measurements.set(element, publish)
  observer.observe(element)
  schedule([element])
  return () => {
    observer?.unobserve(element)
    measurements.delete(element)
    pending.delete(element)
    if (measurements.size === 0) {
      observer?.disconnect()
      observer = undefined
      if (frame !== undefined) cancelAnimationFrame(frame)
      frame = undefined
    }
  }
}
