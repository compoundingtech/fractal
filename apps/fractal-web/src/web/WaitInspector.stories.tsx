import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { colorVars as c } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, userEvent, within } from 'storybook/test'
import { WorkLogV1 } from '@smalltalk/fractal-ui/assistant-ui'
import type { WorkLogCall } from '@smalltalk/fractal-ui/assistant-ui/work-log'
import type { ToolCallItem } from '../conversation/model.ts'
import { prepareTranscriptTurns } from './conversationTranscript.ts'
import { semanticToolDetail } from './semanticTool.tsx'
import { ToolInspector } from './ToolInspector.tsx'

const at = '2026-01-15T12:00:00Z'
const fixtures: readonly ToolCallItem[] = [
  { _tag: 'ToolCall', id: 'example/wait', callId: 'call/wait', name: 'wait', input: { jobs: ['bg/example'] }, status: 'success', callSeen: true, at, result: { content: 'Arrived', isError: false, at, metadata: { jobs: [{ jobId: 'bg/example' }] } } },
  { _tag: 'ToolCall', id: 'example/read-control', callId: 'call/read', name: 'read', input: { path: 'notes/wait-notes.md' }, status: 'success', callSeen: true, at, result: { content: 'A recorded note about waiting.', isError: false, at } },
]
const WaitInspector = () => {
  const [opened, setOpened] = React.useState<WorkLogCall | undefined>()
  const turn = prepareTranscriptTurns(fixtures, { firstTurnComplete: true })[0]!
  return <div {...stylex.props(styles.canvas)}><section style={{ flex: 1 }}><WorkLogV1 turn={turn.work} renderCallDetail={call => { const item = fixtures.find(item => item.id === call.id); return item === undefined ? undefined : semanticToolDetail(item, () => setOpened(call)) }} /></section>{opened !== undefined && <ToolInspector call={opened} width={440} onClose={() => setOpened(undefined)} />}</div>
}
const styles = stylex.create({ canvas: { display: 'flex', gap: 24, backgroundColor: c.canvas, color: c.fg, padding: 24 } })
const meta = { title: 'Fractal Web/Transcript/Wait inspection', component: WaitInspector } satisfies Meta<typeof WaitInspector>
export default meta
type Story = StoryObj<typeof meta>
export const AllStates: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await userEvent.click(canvasElement.querySelector('[data-work-log-summary]')!)
  await expect(canvasElement.querySelector('[data-item-id="example/wait"] [data-tool-kind]')).toHaveAttribute('data-tool-kind', 'wait')
  // Failing control: a filename and result mentioning waiting do not turn a read into a wait.
  await expect(canvasElement.querySelector('[data-item-id="example/read-control"] [data-tool-kind]')).toHaveAttribute('data-tool-kind', 'read')
  await userEvent.click(canvasElement.querySelector('[data-item-id="example/wait"] button')!)
  await userEvent.click(canvas.getByRole('button', { name: 'Open inspector for wait' }))
  await expect(canvasElement.querySelector('[data-tool-inspector]')).toHaveAttribute('data-tool-inspector', 'example/wait')
  await expect(canvasElement.querySelector('[data-typed-tool-result]')).toHaveAttribute('data-typed-tool-result', 'wait')
  await expect(canvasElement.querySelector('[data-tool-input]')).toHaveTextContent('bg/example')
  await userEvent.click(canvas.getByRole('button', { name: 'Close tool detail' }))
  await expect(canvasElement.querySelector('[data-tool-inspector]')).toBeNull()
} }
