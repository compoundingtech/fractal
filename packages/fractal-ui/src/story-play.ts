/** Shared Storybook play helpers for kit stories. */
import { expect, waitFor, within } from 'storybook/test'

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

/** Await the final progressive-history commit, not a wall-clock budget for mounting its batches. */
export async function committedTranscriptHistory(canvasElement: HTMLElement, expectedTurns: number, abortSignal: AbortSignal): Promise<HTMLElement> {
  const canvas = within(canvasElement)
  const controls = canvas.getByTestId('transcript-history-controls')
  let observer: MutationObserver | undefined
  let onAbort: (() => void) | undefined
  try {
    await new Promise<void>((resolve, reject) => {
      const complete = () => {
        if (controls.dataset.state === 'complete') resolve()
      }
      onAbort = () => reject(new Error(`History readiness aborted: expected data-state="complete" and ${expectedTurns} turns; last data-state="${controls.dataset.state}", mounted turns=${canvasElement.querySelectorAll('[data-testid="transcript-turn"]').length}`))
      abortSignal.addEventListener('abort', onAbort, { once: true })
      if (abortSignal.aborted) { onAbort(); return }
      observer = new MutationObserver(complete)
      observer.observe(controls, { attributes: true, attributeFilter: ['data-state'] })
      // Also handle a final commit that preceded observer registration.
      complete()
    })
  } finally {
    observer?.disconnect()
    if (onAbort !== undefined) abortSignal.removeEventListener('abort', onAbort)
  }
  await expect(controls).toHaveAttribute('aria-busy', 'false')
  const viewport = canvas.getByTestId('transcript-scroll')
  await expect(viewport.querySelectorAll('[data-testid="transcript-turn"]')).toHaveLength(expectedTurns)
  return viewport
}
