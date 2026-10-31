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

/** Observe the final history commit; each advancing batch gets the configured async assertion deadline. */
export async function committedTranscriptHistory(canvasElement: HTMLElement, expectedTurns: number, abortSignal: AbortSignal): Promise<HTMLElement> {
  const canvas = within(canvasElement)
  const controls = canvas.getByTestId('transcript-history-controls')
  let observer: MutationObserver | undefined
  let onAbort: (() => void) | undefined
  let deadline: number | undefined
  try {
    await new Promise<void>((resolve, reject) => {
      let mountedTurns = canvasElement.querySelectorAll('[data-testid="transcript-turn"]').length
      const complete = () => {
        if (controls.dataset.state === 'complete') { resolve(); return }
        const nextTurns = canvasElement.querySelectorAll('[data-testid="transcript-turn"]').length
        if (nextTurns > mountedTurns && nextTurns <= expectedTurns) {
          mountedTurns = nextTurns
          window.clearTimeout(deadline)
          deadline = window.setTimeout(() => fail(`stalled for ${timeout}ms`), timeout)
        }
      }
      const fail = (reason: string) => reject(new Error(`History readiness ${reason}: expected data-state="complete" and ${expectedTurns} turns; last data-state="${controls.dataset.state}", mounted turns=${canvasElement.querySelectorAll('[data-testid="transcript-turn"]').length}`))
      onAbort = () => fail('aborted')
      abortSignal.addEventListener('abort', onAbort, { once: true })
      if (abortSignal.aborted) { onAbort(); return }
      const timeout = getConfig().asyncUtilTimeout
      deadline = window.setTimeout(() => fail(`stalled for ${timeout}ms`), timeout)
      observer = new MutationObserver(complete)
      observer.observe(canvasElement, { attributes: true, attributeFilter: ['data-state'], childList: true, subtree: true })
      // Also handle a final commit that preceded observer registration.
      complete()
    })
  } finally {
    observer?.disconnect()
    window.clearTimeout(deadline)
    if (onAbort !== undefined) abortSignal.removeEventListener('abort', onAbort)
  }
  await expect(controls).toHaveAttribute('aria-busy', 'false')
  const viewport = canvas.getByTestId('transcript-scroll')
  await expect(viewport.querySelectorAll('[data-testid="transcript-turn"]')).toHaveLength(expectedTurns)
  return viewport
}
