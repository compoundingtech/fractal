// @vitest-environment jsdom
/** Decided 2026-11-08: a finished turn is one collapsed line, its rows open on click, failures stay in view, and a row's detail never repeats its title and status. */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { EmbraceRuntimeProvider } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceRuntime.tsx'
import { Transcript, type TranscriptTurn } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Transcript.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const now = Date.parse('2026-11-08T12:00:00Z')
const at = new Date(now).toISOString()
const read = (index: number, status: 'success' | 'error' = 'success') => ({
  item: { _tag: 'ToolCall' as const, id: `tool/${index}`, callId: `call/${index}`, name: 'read', input: { path: `file-${index}.ts` }, status, callSeen: true, at, result: { content: `contents ${index}`, isError: status === 'error', at } },
  call: { id: `tool/${index}`, kind: 'read' as const, title: 'Reading information', summary: 'Reading information', status, startedAt: at, endedAt: at, detail: `contents ${index}` },
})
const settled = (calls: readonly ReturnType<typeof read>[]): TranscriptTurn => ({
  id: 'turn-1',
  prompt: { _tag: 'Text', id: 'prompt-1', role: 'user', text: 'Read the files.', attachments: [], streaming: false, at },
  items: calls.map(entry => entry.item),
  work: { calls: calls.map(entry => entry.call), durationMs: 6000, running: false, failed: false, interrupted: false },
})

let root: Root
let container: HTMLDivElement
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
})
afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})
const render = async (turn: TranscriptTurn) => {
  await act(async () => root.render(<EmbraceRuntimeProvider options={{ messages: [turn.prompt!, ...turn.items], isRunning: false, onNew: async () => {} }}>
    <Transcript turns={[turn]} title="Settled" sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} onOpenTool={() => {}} />
  </EmbraceRuntimeProvider>))
  await act(async () => { await Promise.resolve() })
}
const summary = () => container.querySelector<HTMLButtonElement>('[data-testid="work-log"] > button')!
const rows = () => [...container.querySelectorAll<HTMLElement>('[data-testid="work-log"] [data-tool-status]')]

it('shows a finished turn as one collapsed line naming its time and tool count', async () => {
  await render(settled([1, 2, 3, 4, 5].map(index => read(index))))
  expect(summary().textContent).toContain('Worked 6s · 5 tools')
  expect(summary().getAttribute('aria-expanded')).toBe('false')
  expect(rows()).toHaveLength(0)
})

it('opens rows only when clicked, and the open row does not repeat its title and status', async () => {
  await render(settled([read(1), read(2)]))
  await act(async () => summary().click())
  expect(rows()).toHaveLength(2)
  expect(container.querySelector('[data-testid="tool-detail-preview"]')).toBeNull()
  await act(async () => rows()[0]!.querySelector<HTMLButtonElement>('button')!.click())
  const preview = container.querySelector<HTMLElement>('[data-testid="tool-detail-preview"]')!
  expect(preview).not.toBeNull()
  // The row names the call; the detail says its status once, and only the status the row does not show.
  expect(preview.textContent).not.toContain('Reading information')
  expect(preview.querySelector('p')?.textContent).toBe('success')
})

it('keeps a failed tool row in view while the turn is collapsed', async () => {
  await render(settled([read(1), read(2, 'error'), read(3)]))
  expect(summary().getAttribute('aria-expanded')).toBe('false')
  expect(rows().map(row => row.dataset.toolStatus)).toEqual(['error'])
  await act(async () => summary().click())
  expect(rows()).toHaveLength(3)
})
