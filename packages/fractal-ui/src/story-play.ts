/** Shared Storybook play helpers for kit stories. */
import { expect, getConfig, waitFor, within } from 'storybook/test'

/**
 * The anchor's tooltip, resolved through the anchor's own aria-describedby and asserted to have
 * role tooltip, exactly `name` as its accessible name, and to be visible. A document-wide ByRole
 * query re-runs on every mutation and its cost grows with the page: about 300ms per check at 6x
 * CPU on an 811-element tab strip page, where a few checks consumed the whole 1s budget and
 * missed a tooltip that was already mounted. The anchor lookup does not depend on page size.
 */
export async function tooltipFor(anchor: HTMLElement, name: string): Promise<HTMLElement> {
  return waitFor(() => {
    const tooltip = anchor.getAttribute('aria-describedby')?.split(/\s+/).map(id => anchor.ownerDocument.getElementById(id)).find(node => node?.getAttribute('role') === 'tooltip')
    if (tooltip === undefined || tooltip === null) throw new Error(`Missing tooltip for ${name}`)
    expect(tooltip).toHaveAccessibleName(name)
    expect(tooltip).toBeVisible()
    return tooltip
  })
}

/**
 * Observe the final history commit. History advances one batch per animation frame, so a stall is the configured
 * async deadline passing with no new turn, then two more frames with none: one to run a batch and the next to show
 * its commit. A browser that renders no frames gives the viewport no chance to advance (a loaded host can pause its
 * compositor for a second while page timers still run), so the deadline waits for those frames. Frames from before
 * the deadline do not count, so a pause that outlasts it still leaves the first frame after it to commit a batch.
 */
export async function committedTranscriptHistory(canvasElement: HTMLElement, expectedTurns: number, abortSignal: AbortSignal): Promise<HTMLElement> {
  const canvas = within(canvasElement)
  const controls = canvas.getByTestId('transcript-history-controls')
  let observer: MutationObserver | undefined
  let onAbort: (() => void) | undefined
  let deadline: number | undefined
  let frame: number | undefined
  try {
    await new Promise<void>((resolve, reject) => {
      const timeout = getConfig().asyncUtilTimeout
      let mountedTurns = canvasElement.querySelectorAll('[data-testid="transcript-turn"]').length
      let expired = false
      let frames = 0
      const fail = (reason: string) => reject(new Error(`History readiness ${reason}: expected data-state="complete" and ${expectedTurns} turns; last data-state="${controls.dataset.state}", mounted turns=${canvasElement.querySelectorAll('[data-testid="transcript-turn"]').length}`))
      const stalled = () => { if (expired && frames >= 2) fail(`stalled for ${timeout}ms and ${frames} frames`) }
      const arm = () => {
        expired = false
        frames = 0
        window.clearTimeout(deadline)
        deadline = window.setTimeout(() => { expired = true; frames = 0 }, timeout)
      }
      const countFrame = () => {
        frames++
        stalled()
        frame = window.requestAnimationFrame(countFrame)
      }
      const complete = () => {
        if (controls.dataset.state === 'complete') { resolve(); return }
        const nextTurns = canvasElement.querySelectorAll('[data-testid="transcript-turn"]').length
        if (nextTurns > mountedTurns && nextTurns <= expectedTurns) {
          mountedTurns = nextTurns
          arm()
        }
      }
      onAbort = () => fail('aborted')
      abortSignal.addEventListener('abort', onAbort, { once: true })
      if (abortSignal.aborted) { onAbort(); return }
      arm()
      frame = window.requestAnimationFrame(countFrame)
      observer = new MutationObserver(complete)
      observer.observe(canvasElement, { attributes: true, attributeFilter: ['data-state'], childList: true, subtree: true })
      // Also handle a final commit that preceded observer registration.
      complete()
    })
  } finally {
    observer?.disconnect()
    window.clearTimeout(deadline)
    if (frame !== undefined) window.cancelAnimationFrame(frame)
    if (onAbort !== undefined) abortSignal.removeEventListener('abort', onAbort)
  }
  await expect(controls).toHaveAttribute('aria-busy', 'false')
  const viewport = canvas.getByTestId('transcript-scroll')
  await expect(viewport.querySelectorAll('[data-testid="transcript-turn"]')).toHaveLength(expectedTurns)
  return viewport
}
