import * as React from 'react'

/** Retained inactive transcript panes unregister measurements until shown again. */
export const TranscriptActiveContext = React.createContext(true)

/** Each frame reads a bounded prompt slice before publishing any overflow state. */
const measurements = new Map<HTMLElement, (overflows: boolean) => void>()
const pending = new Set<HTMLElement>()
const batchSize = 16
let observer: ResizeObserver | undefined
let frame: number | undefined

function flush() {
  frame = undefined
  const results: { element: HTMLElement; publish: (overflows: boolean) => void; overflows: boolean }[] = []
  for (const element of pending) {
    pending.delete(element)
    const publish = measurements.get(element)
    if (publish !== undefined) results.push({ element, publish, overflows: element.scrollHeight > element.clientHeight })
    if (results.length === batchSize) break
  }
  // Publishing can unmount rows. Never deliver their queued measurements afterward.
  for (const { element, publish, overflows } of results) if (measurements.get(element) === publish) publish(overflows)
  if (pending.size > 0 && frame === undefined) frame = requestAnimationFrame(flush)
}

function schedule(elements: readonly Element[]) {
  for (const element of elements) if (measurements.has(element as HTMLElement)) pending.add(element as HTMLElement)
  if (frame === undefined && pending.size > 0) frame = requestAnimationFrame(flush)
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
    if (pending.size === 0 && frame !== undefined) {
      cancelAnimationFrame(frame)
      frame = undefined
    }
    if (measurements.size === 0) {
      observer?.disconnect()
      observer = undefined
    }
  }
}
