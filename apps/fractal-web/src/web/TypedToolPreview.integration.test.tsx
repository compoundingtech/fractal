// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import type { WorkLogCall } from '@fractal/ui/assistant-ui/work-log'
import { TypedToolPreview } from './TypedToolPreview.tsx'
vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (values: unknown) => values, createTheme: () => ({}), keyframes: () => '', props: () => ({}) }))
const call: WorkLogCall = { id: 'example/task', title: 'scout', toolName: 'task', kind: 'subagent', status: 'success', startedAt: '', input: { tasks: [{ name: 'ExampleReview', agent: 'scout', task: '# Review\n\nInspect **readable fields**.' }] }, output: [{ type: 'text', text: 'Spawned `ExampleReview`.\n\nResults **auto-deliver** at `agent://ExampleWorker`.' }], metadata: { results: [{ name: 'ExampleReview', state: 'pending' }] } }

it('renders the recorded task result as Markdown and keeps the host detail one press away', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const container = document.createElement('div'), root = createRoot(container)
  const open = vi.fn()
  try {
    await act(async () => root.render(<TypedToolPreview call={call} onOpen={open} />))
    const prose = container.querySelector('[aria-label="Recorded result body"]')!
    expect(prose.querySelector('strong')?.textContent).toBe('auto-deliver')
    expect(prose.querySelector('code')?.textContent).toBe('ExampleReview')
    expect(prose.querySelector('em')).toBeNull()
    expect(prose.querySelector('pre')).toBeNull()
    expect(container.textContent).toContain('State at task launch: pending')
    const button = container.querySelector<HTMLButtonElement>('button[aria-label="Open inspector for task"]')!
    await act(async () => button.click())
    expect(open).toHaveBeenCalledOnce()
  } finally { await act(async () => root.unmount()); vi.unstubAllGlobals() }
})
