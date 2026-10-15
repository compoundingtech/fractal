import type { Meta, StoryObj } from '@storybook/react'
import { expect, userEvent, within } from 'storybook/test'
import { WorkLogV1 } from '@smalltalk/fractal-ui/assistant-ui'
import type { ToolCallItem } from '../conversation/model.ts'
import { prepareTranscriptTurns } from './conversationTranscript.ts'
import { semanticToolDetail } from './semanticTool.tsx'
const at = '2026-01-15T12:00:00Z'
const fixtures: readonly ToolCallItem[] = [
  { _tag:'ToolCall', id:'example/message', callId:'call/message', name:'write', input:{path:'agent://all',content:'The review is ready.'},status:'success',callSeen:true,at,result:{content:'sent',isError:false,at,metadata:{message:{to:['Builder','Reviewer'],receipts:[{to:'Builder',outcome:'injected'},{to:'Reviewer',outcome:'queued'}]}}}},
  { _tag:'ToolCall', id:'example/wait', callId:'call/wait', name:'wait', input:{jobs:['bg_example']},status:'success',callSeen:true,at,result:{content:'arrived',isError:false,at,metadata:{jobs:[{jobId:'bg_example'}],waited:12}}},
  { _tag:'ToolCall', id:'example/skip', callId:'call/skip', name:'bash', input:{command:'printf ready'},status:'success',skipped:true,callSeen:true,at,result:{content:'Interrupted before execution.',isError:false,at}},
  { _tag:'ToolCall', id:'example/file-control', callId:'call/file', name:'write', input:{path:'notes/agent-plan.md',content:'Review notes'},status:'success',callSeen:true,at,result:{content:'written',isError:false,at}},
]
const SemanticTools = () => {
  const turn = prepareTranscriptTurns(fixtures,{firstTurnComplete:true})[0]!
  return <WorkLogV1 turn={turn.work} renderCallDetail={call=>{const item=fixtures.find(item=>item.id===call.id);return item===undefined?undefined:semanticToolDetail(item)}} />
}
const meta = {title:'Fractal Web/Transcript/Semantic tools',component:SemanticTools} satisfies Meta<typeof SemanticTools>
export default meta
type Story=StoryObj<typeof meta>
export const AllStates:Story={play:async({canvasElement})=>{
  const canvas=within(canvasElement)
  await userEvent.click(canvasElement.querySelector('[data-work-log-summary]')!)
  const wait=canvasElement.querySelector('[data-item-id="example/wait"]')!
  await expect(wait.querySelector('[data-tool-kind]')).toHaveTextContent('wait')
  await userEvent.click(wait.querySelector('button')!)
  await expect(canvas.getByTestId('wait-row')).toHaveTextContent('Wait finished')
  const sent=canvasElement.querySelector('[data-item-id="example/message"]')!
  await expect(sent.querySelector('[data-tool-kind]')).toHaveTextContent('message')
  await userEvent.click(sent.querySelector('button')!)
  await expect(canvas.getAllByTestId('outgoing-message-card')).toHaveLength(1)
  await expect(canvas.getByTestId('outgoing-message-card')).toHaveTextContent('→ 2 seats')
  await expect(canvas.getByTestId('outgoing-message-card')).toHaveTextContent('Reviewer · queued')
  await expect(canvasElement.querySelector('[data-item-id="example/skip"]')).toHaveTextContent('Skipped')
  await expect(canvas.queryByText('Run failed')).toBeNull()
  // Failing control: an ordinary file whose name contains agent must remain a change.
  await expect(canvasElement.querySelector('[data-item-id="example/file-control"] [data-tool-kind]')).toHaveTextContent('change')
}}
