import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fn, userEvent, within } from 'storybook/test'
import type { WorkLogCall } from '@fractal/ui/assistant-ui/work-log'
import { colorVars as c } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { TypedToolPreview } from './TypedToolPreview.tsx'

const base = { title: 'scout', status: 'success', kind: 'subagent', startedAt: '2026-01-15T12:00:00Z', toolName: 'task' } as const
const defaultTask: WorkLogCall = { ...base, id: 'example/default-task', input: { tasks: [{ name: 'ExampleReview', agent: 'scout', task: '#### Review\n\nInspect **readable fields** and `agent://ExampleWorker`.\n\n- Preserve evidence' }] }, metadata: { progress: [{id:'ExampleReview',agent:'scout',status:'pending'}], results: [] }, output: [{ type: 'text', text: 'Spawned agent `ExampleReview` (job `ExampleReview`).\n\nResults **auto-deliver** at `agent://ExampleWorker`.' }] }
const unknownTask: WorkLogCall = { ...base, id: 'example/opaque-task', input: { tasks: [{ task: 42 }] }, output: { future_only: 'Opaque native field' } }
const RecordedTaskResult = ({ onOpen }: { readonly onOpen: () => void }) => <div {...stylex.props(styles.canvas)}>
  <section data-testid="task-default"><TypedToolPreview call={defaultTask} onOpen={onOpen} /></section>
  <section data-testid="task-opaque"><TypedToolPreview call={unknownTask} onOpen={onOpen} /></section>
</div>
const styles = stylex.create({canvas:{display:'flex',flexDirection:'column',gap:24,backgroundColor:c.canvas,color:c.fg,padding:24,minWidth:560}})
const meta = { title: 'Fractal Web/Transcript/Readable task results', component: RecordedTaskResult, args: { onOpen: fn() } } satisfies Meta<typeof RecordedTaskResult>
export default meta
type Story = StoryObj<typeof meta>
export const AllStates: Story = { play: async ({ canvasElement, args }) => {
  const recorded = within(canvasElement).getByTestId('task-default')
  const inline = recorded.querySelector('[aria-label="Recorded result body"]')!
  await expect(inline.querySelector('pre')).toBeNull()
  await expect(inline.querySelector('em')).toBeNull()
  await expect(within(inline as HTMLElement).getByText('auto-deliver').tagName).toBe('STRONG')
  await expect(inline).not.toHaveTextContent('\\n\\nResults')
  await expect(recorded.querySelector('[aria-label="Recorded tasks"] strong')).toHaveTextContent('ExampleReview')
  await expect(recorded.querySelector('[aria-label="Recorded tasks"]')).toHaveTextContent('Agent type: scout')
  await expect(recorded).toHaveTextContent('State at task launch: pending')
  await userEvent.click(within(recorded).getByRole('button', { name: 'Open inspector for task' }))
  await expect(args.onOpen).toHaveBeenCalledOnce()
  // Failing control: an opaque result stays explicit raw evidence, never a typed fact.
  const opaque = within(canvasElement).getByTestId('task-opaque')
  const primary = opaque.cloneNode(true) as HTMLElement
  for (const details of primary.querySelectorAll('details')) details.remove()
  await expect(primary).not.toHaveTextContent('Opaque native field')
  await expect(primary).not.toHaveTextContent('Completed')
  await userEvent.click(within(opaque).getByText('Raw evidence'))
  await expect(opaque.querySelector('[data-typed-tool-result] details pre')).toBeVisible()
} }
