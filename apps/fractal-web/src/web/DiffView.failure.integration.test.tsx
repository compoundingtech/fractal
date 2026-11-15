// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { DiffView } from '../../../../packages/fractal-ui/src/assistant-ui/composition/DiffView.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const failedImport = vi.hoisted(() => ({ attempts: 0, failure: new TypeError('Failed to fetch dynamically imported module: DiffView.pierre.js') }))
// The renderer chunk itself rejects: DiffView's real lazy import of DiffView.pierre, not a substituted renderer.
vi.mock('../../../../packages/fractal-ui/src/assistant-ui/composition/DiffView.pierre.tsx', async () => {
  failedImport.attempts++
  throw failedImport.failure
})

const patch = (line: string) => `--- a/src/x.ts\n+++ b/src/x.ts\n@@ -1,2 +1,2 @@\n keep\n-old\n+${line}\n`
// The loader may wrap a failing module (vitest names the mock); the import's own failure stays in the cause chain.
const causes = (error: unknown): unknown[] => error instanceof Error ? [error, ...causes(error.cause)] : error === undefined ? [] : [error]

it('a rejected renderer import keeps the plain lines under a visible notice and reports the load error once per view', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => setTimeout(() => callback(0), 0))
  vi.stubGlobal('cancelAnimationFrame', () => {})
  const caught: unknown[] = []
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container, { onCaughtError: error => { caught.push(error) } })
  const show = (line: string) => act(async () => root.render(<><DiffView patch={patch(line)} testId="first" /><DiffView patch={patch(line)} testId="second" /></>))
  const view = (testId: string) => container.querySelector<HTMLElement>(`[data-testid="${testId}"]`)!
  try {
    await show('new')
    await act(async () => { await vi.waitFor(() => expect(view('second').dataset.diffState).toBe('plain-failed')) })
    // Both views share one import request.
    expect(failedImport.attempts).toBe(1)
    // React reported the load error through the root, once per view, with the rejected import as its cause.
    expect(caught).toHaveLength(2)
    const [reported] = caught
    expect(caught[1]).toBe(reported)
    expect(reported).toBeInstanceOf(Error)
    const { message } = reported as Error
    expect(message.startsWith('The diff renderer could not load: ')).toBe(true)
    expect(causes(reported)).toContain(failedImport.failure)
    for (const testId of ['first', 'second']) {
      const host = view(testId)
      expect(host.dataset.diffState).toBe('plain-failed')
      expect(host.hasAttribute('aria-busy')).toBe(false)
      expect(host.querySelector('[role="alert"][data-diff-error]')?.textContent).toBe(`${message}. Showing plain lines.`)
      expect(host.querySelector('[data-diff-plain]')?.textContent).toBe(' keep\n-old\n+new\n')
    }
    // A new patch updates the plain lines under the same notice: no retry and no second report.
    await show('newer')
    await act(async () => { await vi.waitFor(() => expect(view('first').querySelector('[data-diff-plain]')?.textContent).toBe(' keep\n-old\n+newer\n')) })
    expect(view('first').querySelector('[role="alert"][data-diff-error]')).not.toBeNull()
    expect(view('first').dataset.diffState).toBe('plain-failed')
    expect(failedImport.attempts).toBe(1)
    expect(caught).toHaveLength(2)
  } finally {
    await act(async () => root.unmount())
    container.remove()
    vi.unstubAllGlobals()
  }
})
