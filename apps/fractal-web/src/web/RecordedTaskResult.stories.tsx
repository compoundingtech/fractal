import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, userEvent, within } from 'storybook/test'
import type { WorkLogCall } from '@smalltalk/fractal-ui/assistant-ui/work-log'
import { colorVars as c } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { ToolInspector } from './ToolInspector.tsx'
import { TaskResultPreview } from './TaskResultPreview.tsx'

const base = { title: 'scout', status: 'success', kind: 'subagent', startedAt: '2026-01-15T12:00:00Z', toolName: 'task' } as const
const defaultTask: WorkLogCall = { ...base, id: 'example/default-task', input: { tasks: [{ name: 'ExampleReview', agent: 'scout', task: '#### Review\n\nInspect **readable fields** and `agent://ExampleWorker`.\n\n- Preserve evidence' }] }, metadata: { progress: [{id:'ExampleReview',agent:'scout',status:'pending'}], results: [] }, output: [{ type: 'text', text: 'Spawned agent `ExampleReview` (job `ExampleReview`).\n\nResults **auto-deliver** at `agent://ExampleWorker`.' }] }
const unknownTask: WorkLogCall = { ...base, id: 'example/opaque-task', input: { tasks: [{ task: 42 }] }, output: { future_only: 'Opaque native field' } }
const RecordedTaskResult = () => {
  const [inspect, setInspect] = React.useState(false)
  const [control, setControl] = React.useState(false)
  return <div {...stylex.props(styles.canvas)}><Button onPress={() => {setControl(true);setInspect(true)}}>Inspect opaque control</Button>{inspect ? <ToolInspector call={control ? unknownTask : defaultTask} onClose={() => setInspect(false)} /> : <TaskResultPreview call={defaultTask} onOpen={() => setInspect(true)} />}</div>
}
const styles = stylex.create({canvas:{backgroundColor:c.canvas,color:c.fg,padding:24,minWidth:560}})
const meta = { title: 'Fractal Web/Transcript/Readable task results', component: RecordedTaskResult } satisfies Meta<typeof RecordedTaskResult>
export default meta
type Story = StoryObj<typeof meta>
export const AllStates: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const inline = canvasElement.querySelector('[aria-label="Recorded result body"]')!
  await expect(inline.querySelector('pre')).toBeNull()
  await expect(inline.querySelector('em')).toBeNull()
  await expect(inline.querySelectorAll('code')[2]).toHaveTextContent('agent://ExampleWorker')
  await userEvent.click(canvas.getByRole('button',{name:'Open inspector for task'}))
  const recorded = canvasElement.querySelector('[data-tool-inspector="example/default-task"]')!
  const primary = recorded.cloneNode(true) as HTMLElement
  for (const details of primary.querySelectorAll('details')) details.remove()
  await expect(within(primary).getByRole('heading',{level:2})).toHaveTextContent('ExampleReview')
  const input = primary.querySelector('[data-task-invocation]')!
  await expect(input).toHaveTextContent('Agent typescout')
  await expect(within(input as HTMLElement).getByText('readable fields').tagName).toBe('STRONG')
  await expect(within(input as HTMLElement).getByRole('heading',{name:'Review'})).toHaveTextContent('Review')
  await expect(input).not.toHaveTextContent('\\n')
  await expect(primary).toHaveTextContent('State at task launch: pending')
  await expect(primary.querySelector('[aria-label="Recorded tasks"] strong')).toHaveTextContent('ExampleReview')
  await expect(primary.querySelector('[aria-label="Recorded tasks"]')).toHaveTextContent('Agent type: scout')
  const prose = primary.querySelector('[aria-label="Recorded result body"]')!
  await expect(within(prose as HTMLElement).getByText('auto-deliver').tagName).toBe('STRONG')
  await expect(prose).not.toHaveTextContent('\\n\\nResults')
  // Failing control: malformed requests and opaque results remain explicit raw evidence.
  await userEvent.click(canvas.getByRole('button',{name:'Inspect opaque control'}))
  const opaque = canvasElement.querySelector('[data-tool-inspector="example/opaque-task"]')!
  await expect(opaque.querySelector('[data-task-invocation]')).toBeNull()
  const opaquePrimary = opaque.cloneNode(true) as HTMLElement
  for (const details of opaquePrimary.querySelectorAll('details')) details.remove()
  await expect(opaquePrimary).not.toHaveTextContent('Opaque native field')
  await expect(opaquePrimary).not.toHaveTextContent('Completed')
  await userEvent.click(within(opaque as HTMLElement).getByText('Raw evidence'))
  await expect(opaque.querySelector('[data-typed-tool-result] details pre')).toBeVisible()
} }
