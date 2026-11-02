import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'
import type { WorkLogCall } from '@fractal/ui/assistant-ui/work-log'
import { toolKindFor } from '../conversation/toolClassification.ts'
import { TypedToolResult } from './TypedToolResult.tsx'

const base = { id: 'example-call', title: 'Recorded result', status: 'success', startedAt: '2026-01-01T00:00:00Z' } as const
const examples: readonly WorkLogCall[] = [
  { ...base, id: 'search', kind: 'search', toolName: 'grep', input: { pattern: 'agent.*', path: 'src/example.ts' }, output: { files: ['src/example.ts'], matches: [{ path: 'src/example.ts', line: 12, text: 'agent.start()' }] } },
  { ...base, id: 'web', kind: 'web', toolName: 'web_search', input: { query: 'observed web query' }, output: { content: [{ type: 'text', text: '## Recorded findings\n\n[Example source](https://example.org/source)' }] }, metadata: { sources: [{ title: 'Example documentation', url: 'https://example.org/source', snippet: 'A recorded source snippet.' }] } },
  { ...base, id: 'tasks', kind: 'subagent', view: { tasks: [{ name: 'Example index task', state: 'running', ref: 'agent://example-index' }] } },
  { ...base, id: 'wait', kind: 'wait', status: 'running', input: { handles: ['agent://example-index'] }, metadata: { arrived: ['agent://example-review'] } },
  { ...base, id: 'interrupted', kind: 'wait', status: 'interrupted', input: { handles: ['agent://example-pending'] } },
  { ...base, id: 'skipped', kind: 'web', status: 'error', skipped: true },
  { ...base, id: 'unknown', toolName: 'my_agent_search', kind: toolKindFor('my_agent_search'), output: { envelope: 'Only disclosed on demand' } },
]
const meta = { title: 'Fractal Web/Transcript/Typed tool results', component: TypedToolResult, args: { call: examples[0]! } } satisfies Meta<typeof TypedToolResult>
export default meta
type Story = StoryObj<typeof meta>
export const AllStates: Story = {
  render: () => <div>{examples.map(call => <section key={call.id} data-testid={`result-${call.id}`}><h2>{call.id}</h2><TypedToolResult call={call} /></section>)}</div>,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(within(canvas.getByTestId('result-search')).getByText('agent.*')).toBeVisible()
    await expect(within(canvas.getByTestId('result-web')).getByText('observed web query')).toBeVisible()
    await expect(within(canvas.getByTestId('result-web')).getByRole('link', { name: 'https://example.org/source' })).toHaveAttribute('href', 'https://example.org/source')
    await expect(within(canvas.getByTestId('result-tasks')).getByText('Example index task')).toBeVisible()
    await expect(within(canvas.getByTestId('result-wait')).getByText('Running')).toBeVisible()
    await expect(within(canvas.getByTestId('result-interrupted')).getByText('Interrupted')).toBeVisible()
    await expect(within(canvas.getByTestId('result-skipped')).getByText('Skipped')).toBeVisible()
    // Failing controls: a name containing agent is not a task; wait is not a read.
    await expect(canvas.getByTestId('result-unknown').querySelector('[data-typed-tool-result]')).toHaveAttribute('data-typed-tool-result', 'unknown')
    await expect(within(canvas.getByTestId('result-unknown')).queryByText('Tasks')).toBeNull()
    await expect(canvas.getByTestId('result-wait').querySelector('[data-typed-tool-result]')).toHaveAttribute('data-typed-tool-result', 'wait')
    await expect(within(canvas.getByTestId('result-skipped')).queryByText('Failed')).toBeNull()
    await expect(within(canvas.getByTestId('result-unknown')).getByText(/Only disclosed on demand/)).not.toBeVisible()
  },
}
