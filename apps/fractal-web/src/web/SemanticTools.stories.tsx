import type { Meta, StoryObj } from '@storybook/react-vite'
import * as stylex from '@stylexjs/stylex'
import { expect, within } from 'storybook/test'
import { colorVars as c } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { ToolCallItem } from '../conversation/model.ts'
import { prepareTranscriptTurns } from './conversationTranscript.ts'
import { semanticToolDetail } from './semanticTool.tsx'
const at = '2026-01-15T12:00:00Z'
const fixtures: readonly ToolCallItem[] = [
  { _tag:'ToolCall', id:'example/message', callId:'call/message', name:'write', input:{path:'agent://all',content:'The review is ready.'},status:'success',callSeen:true,at,result:{content:'sent',isError:false,at,metadata:{message:{to:['Builder','Reviewer'],receipts:[{to:'Builder',outcome:'injected'},{to:'Reviewer',outcome:'queued'}]}}}},
  { _tag:'ToolCall', id:'example/send', callId:'call/send', name:'bash', input:{command:'st conversations send agent/example/Reviewer --from agent/example/Builder --idempotency-key example-send-1 --subject "Recorded review" --body "Please inspect the **recorded review**." </dev/null 2>&1 | tail -1'},status:'success',callSeen:true,at,result:{content:'Message sent.',isError:false,at}},
  { _tag:'ToolCall', id:'example/wait', callId:'call/wait', name:'wait', input:{jobs:['bg_example']},status:'success',callSeen:true,at,result:{content:'arrived',isError:false,at,metadata:{jobs:[{jobId:'bg_example',status:'completed'}],waited:12}}},
  { _tag:'ToolCall', id:'example/skip', callId:'call/skip', name:'bash', input:{command:'printf ready'},status:'success',skipped:true,callSeen:true,at,result:{content:'Interrupted before execution.',isError:false,at}},
  { _tag:'ToolCall', id:'example/file-control', callId:'call/file', name:'write', input:{path:'notes/agent-plan.md',content:'Review notes'},status:'success',callSeen:true,at,result:{content:'written',isError:false,at}},
  { _tag:'ToolCall', id:'example/mention-control', callId:'call/mention', name:'bash', input:{command:'printf "%s" "st conversations send agent/example/Reviewer --body mentioned"'},status:'success',callSeen:true,at,result:{content:'Recorded mention.',isError:false,at}},
]
/** The details the live transcript supplies through `renderToolDetail`, one per recorded call. */
const SemanticTools = () => {
  const turn = prepareTranscriptTurns(fixtures, { firstTurnComplete: true })[0]!
  return <div {...stylex.props(styles.canvas)}>{turn.work.calls.map(call => {
    const item = fixtures.find(item => item.id === call.id)!
    return <section key={call.id} data-item-id={call.id} data-call-kind={call.kind} aria-label={call.title}>{semanticToolDetail(item)}</section>
  })}</div>
}
const styles = stylex.create({ canvas: { display: 'flex', flexDirection: 'column', gap: 16, backgroundColor: c.canvas, color: c.fg, padding: 24, minWidth: 560 } })
const meta = { title: 'Fractal Web/Transcript/Semantic tools', component: SemanticTools } satisfies Meta<typeof SemanticTools>
export default meta
type Story = StoryObj<typeof meta>
export const AllStates: Story = { play: async ({ canvasElement }) => {
  const row = (id: string) => canvasElement.querySelector<HTMLElement>(`[data-item-id="${id}"]`)!
  await expect(row('example/wait')).toHaveAttribute('data-call-kind', 'wait')
  await expect(within(row('example/wait')).getByTestId('wait-row')).toHaveTextContent('Wait finished')
  await expect(row('example/message')).toHaveAttribute('data-call-kind', 'message')
  await expect(within(row('example/message')).getByTestId('outgoing-message-card')).toHaveTextContent('→ 2 seats')
  await expect(within(row('example/message')).getByTestId('outgoing-message-card')).toHaveTextContent('Reviewer · queued')
  const send = within(row('example/send')).getByTestId('outgoing-message-card')
  await expect(row('example/send')).toHaveAttribute('data-call-kind', 'message')
  await expect(send.querySelector('[data-message-subject]')).toHaveTextContent('Recorded review')
  await expect(send.querySelector('[data-testid="markdown"] strong')).toHaveTextContent('recorded review')
  await expect(send).not.toHaveTextContent('st conversations send')
  await expect(row('example/skip')).toHaveAttribute('aria-label', 'Skipped')
  await expect(within(row('example/skip')).getByTestId('skipped-tool')).toBeVisible()
  // Failing controls: an ordinary file containing agent stays an edit; a printed command never sends.
  await expect(row('example/file-control')).toHaveAttribute('data-call-kind', 'edit')
  await expect(row('example/mention-control')).toHaveAttribute('data-call-kind', 'run')
  await expect(canvasElement.querySelectorAll('[data-testid="outgoing-message-card"]')).toHaveLength(2)
} }
