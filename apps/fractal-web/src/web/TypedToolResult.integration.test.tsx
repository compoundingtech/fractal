// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import type { WorkLogCall } from '@fractal/ui/assistant-ui/work-log'
import { TypedToolResult } from './TypedToolResult.tsx'
vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (values: unknown) => values, createTheme: () => ({}), keyframes: () => '', props: () => ({}) }))
const call: WorkLogCall = { id: 'example/task', title: 'scout', toolName: 'task', kind: 'subagent', status: 'success', startedAt: '', input: { tasks: [{ name: 'ExampleReview', agent: 'scout', task: '# Review\n\nInspect **readable fields**.' }] }, output: [{ type: 'text', text: 'Spawned `ExampleReview`.\n\nResults **auto-deliver** at `agent://ExampleWorker`.' }], metadata: { results: [{ name: 'ExampleReview', state: 'pending' }] } }

it('renders the recorded task result as Markdown, inline, with nothing to open elsewhere', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const container = document.createElement('div'), root = createRoot(container)
  try {
    await act(async () => root.render(<TypedToolResult call={call} />))
    const prose = container.querySelector('[aria-label="Recorded result body"]')!
    expect(prose.querySelector('strong')?.textContent).toBe('auto-deliver')
    expect(prose.querySelector('code')?.textContent).toBe('ExampleReview')
    expect(prose.querySelector('em')).toBeNull()
    expect(prose.querySelector('pre')).toBeNull()
    expect(container.textContent).toContain('State at task launch: pending')
    expect(container.querySelector('button[aria-label^="Open inspector"]')).toBeNull()
  } finally { await act(async () => root.unmount()); vi.unstubAllGlobals() }
})
