/** Shared Storybook play helpers for kit stories. */
import { expect, getConfig, within } from 'storybook/test'

/**
 * The anchor's tooltip, resolved through the anchor's own aria-describedby and asserted to have
 * role tooltip, exactly `name` as its accessible name, and to be visible. A document-wide ByRole
 * query re-runs on every mutation and its cost grows with the page: about 300ms per check at 6x
 * CPU on an 811-element tab strip page, where a few checks consumed the whole 1s budget and
 * missed a tooltip that was already mounted. The anchor lookup does not depend on page size.
 */
export async function tooltipFor(anchor: HTMLElement, name: string): Promise<HTMLElement> {
  return renderedWithin(() => {
    const tooltip = anchor.getAttribute('aria-describedby')?.split(/\s+/).map(id => anchor.ownerDocument.getElementById(id)).find(node => node?.getAttribute('role') === 'tooltip')
    if (tooltip === undefined || tooltip === null) throw new Error(`Missing tooltip for ${name}`)
    expect(tooltip).toHaveAccessibleName(name)
    expect(tooltip).toBeVisible()
    return tooltip
  })
}

/**
 * `check`'s value once it stops throwing, re-run on every page mutation, frame and 50ms, like waitFor. It fails with
 * the last error once the configured async deadline has passed and five frames have rendered since. A loaded host can
 * pause the compositor for a second while page timers still run, and a disclosure opened from a frame callback (the
 * tab strip's keyboard tooltip) cannot appear until frames resume: the first frame after the pause opens it, RAC
 * places the hidden overlay within the next frame or two, and a later frame observes it. Frames from before the
 * deadline do not count.
 */
export async function renderedWithin<T>(check: () => T): Promise<T> {
  let observer: MutationObserver | undefined
  let deadline: number | undefined
  let poll: number | undefined
  let frame: number | undefined
  try {
    return await new Promise<T>((resolve, reject) => {
      let expired = false
      let frames = 0
      let failure: unknown
      const attempt = () => {
        try { resolve(check()); return true } catch (error) { failure = error; return false }
      }
      const countFrame = () => {
        frames++
        if (attempt()) return
        if (expired && frames >= 5) reject(failure)
        else frame = window.requestAnimationFrame(countFrame)
      }
      if (attempt()) return
      deadline = window.setTimeout(() => { expired = true; frames = 0 }, getConfig().asyncUtilTimeout)
      poll = window.setInterval(attempt, 50)
      frame = window.requestAnimationFrame(countFrame)
      observer = new MutationObserver(attempt)
      observer.observe(document.documentElement, { attributes: true, childList: true, characterData: true, subtree: true })
    })
  } finally {
    observer?.disconnect()
    window.clearTimeout(deadline)
    window.clearInterval(poll)
    if (frame !== undefined) window.cancelAnimationFrame(frame)
  }
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
