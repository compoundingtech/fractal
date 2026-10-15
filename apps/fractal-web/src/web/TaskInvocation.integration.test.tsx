// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { ToolInspector } from './ToolInspector.tsx'
import { TaskResultPreview } from './TaskResultPreview.tsx'
import { recordedTaskRequests } from './typedToolResult.ts'
import type { WorkLogCall } from '@smalltalk/fractal-ui/assistant-ui/work-log'
vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (values: unknown) => values, createTheme: () => ({}), keyframes: () => '', props: () => ({}) }))
const call: WorkLogCall = { id: 'example/task', title: 'scout', toolName: 'task', kind: 'subagent', status: 'success', startedAt: '', input: { tasks: [{ name: 'ExampleReview', agent: 'scout', task: '# Review\n\nInspect **readable fields** and `agent://ExampleWorker`.\n\n- Preserve evidence' }] }, output: [{type:'text',text:'Spawned `ExampleReview`.\n\nResults **auto-deliver** at `agent://ExampleWorker`.'}], metadata: { results: [{name:'ExampleReview',state:'pending'}] } }
it('renders named task requests as fields and Markdown, and labels the spawn snapshot', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const container = document.createElement('div'), root = createRoot(container)
  try {
    await act(async () => root.render(<ToolInspector call={call} onClose={() => {}} />))
    expect(container.querySelector('h2')?.textContent).toBe('ExampleReview')
    const input = container.querySelector('[data-task-invocation]')!
    const primary = input.cloneNode(true) as HTMLElement
    primary.querySelector('details')?.remove()
    expect(primary.querySelector('h1')?.textContent).toBe('Review')
    expect(primary.querySelector('strong')?.textContent).toBe('readable fields')
    expect(primary.querySelector('code')?.textContent).toBe('agent://ExampleWorker')
    expect(primary.textContent).toContain('Agent typescout')
    expect(primary.textContent).not.toContain('\\n')
    expect(container.textContent).toContain('State at task launch: pending')
    expect(input.querySelector('details')?.open).toBe(false)
    await act(async () => root.render(<TaskResultPreview call={call} onOpen={() => {}} />))
    const prose = container.querySelector('[aria-label="Recorded result body"]')!
    expect(prose.querySelector('strong')?.textContent).toBe('auto-deliver')
    expect(prose.querySelector('code')?.textContent).toBe('ExampleReview')
    expect(prose.querySelector('em')).toBeNull()
    expect(prose.querySelector('pre')).toBeNull()
  } finally { await act(async () => root.unmount()); vi.unstubAllGlobals() }
})
it('keeps malformed task arrays and prose controls out of typed invocation fields', () => {
  expect(recordedTaskRequests({tasks:[{name:'Invalid',task:42}]})).toBeUndefined()
  expect(recordedTaskRequests('A note mentioning task and agent.')).toBeUndefined()
  expect(recordedTaskRequests({tasks:[{name:'Valid',task:'Read it.'},'opaque']})).toBeUndefined()
})
