import type { Meta, StoryObj } from '@storybook/react'
import * as React from 'react'
import type { WorkLogCall } from '@smalltalk/fractal-ui/assistant-ui/work-log'
import { ToolInspector } from './ToolInspector.tsx'
import * as stylex from '@stylexjs/stylex'
import { colorVars as c } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { expect, userEvent, within } from 'storybook/test'
import { WorkLogV1 } from '@smalltalk/fractal-ui/assistant-ui'
import type { ToolCallItem } from '../conversation/model.ts'
import { prepareTranscriptTurns } from './conversationTranscript.ts'
import { semanticToolDetail } from './semanticTool.tsx'
const at = '2026-01-15T12:00:00Z'
const fixtures: readonly ToolCallItem[] = [
  { _tag:'ToolCall',id:'example/send',callId:'call/send',name:'bash',input:{command:'st conversations send agent/example/Reviewer --from agent/example/Builder --idempotency-key example-send-1 --subject "Recorded review" --body "Please inspect the **recorded review**." </dev/null 2>&1 | tail -1'},status:'success',callSeen:true,at,result:{content:'Message sent.',isError:false,at} },
  { _tag:'ToolCall',id:'example/mention-control',callId:'call/mention',name:'bash',input:{command:'printf "%s" "st conversations send agent/example/Reviewer --body mentioned"'},status:'success',callSeen:true,at,result:{content:'Recorded mention.',isError:false,at} },
]
const RecordedMessageSend = () => {
  const [opened, setOpened] = React.useState<WorkLogCall>()
  const turn = prepareTranscriptTurns(fixtures,{firstTurnComplete:true})[0]!
  return <div {...stylex.props(styles.canvas)}><WorkLogV1 turn={turn.work} renderCallDetail={call => { const item = fixtures.find(item => item.id===call.id); return item===undefined?undefined:semanticToolDetail(item, () => setOpened(call)) }} />{opened !== undefined && <ToolInspector call={opened} onClose={() => setOpened(undefined)} />}</div>
}
const styles = stylex.create({canvas:{backgroundColor:c.canvas,color:c.fg,padding:24,minWidth:560}})
const meta = {title:'Fractal Web/Transcript/Recorded message sends',component:RecordedMessageSend} satisfies Meta<typeof RecordedMessageSend>
export default meta
type Story = StoryObj<typeof meta>
export const AllStates: Story = {play:async({canvasElement}) => {
  await userEvent.click(canvasElement.querySelector('[data-work-log-summary]')!)
  const send = canvasElement.querySelector('[data-item-id="example/send"]')!
  await expect(send.querySelector('[data-tool-kind]')).toHaveAttribute('data-tool-kind','message')
  await userEvent.click(send.querySelector('button')!)
  const card = within(canvasElement).getByTestId('outgoing-message-card')
  await expect(card).toHaveTextContent('Reviewer')
  await expect(card.querySelector('[data-message-subject]')).toHaveTextContent('Recorded review')
  await expect(card.querySelector('[data-testid="markdown"] strong')).toHaveTextContent('recorded review')
  // Failing control: quoting the command in a printing operation does not send a message.
  await expect(canvasElement.querySelector('[data-item-id="example/mention-control"] [data-tool-kind]')).toHaveAttribute('data-tool-kind','run')
  await expect(canvasElement.querySelectorAll('[data-testid="outgoing-message-card"]')).toHaveLength(1)
  await userEvent.click(within(card).getByRole('button',{name:'Open inspector for message'}))
  const inspector = canvasElement.querySelector('[data-tool-inspector]')!
  await expect(inspector).toHaveAttribute('data-result-view','message')
  const primary = inspector.cloneNode(true) as HTMLElement
  for(const details of primary.querySelectorAll('details')) details.remove()
  await expect(primary).not.toHaveTextContent('st conversations send')
  await expect(primary).toHaveTextContent('Recorded review')
  await userEvent.click(within(inspector as HTMLElement).getByRole('button',{name:'Close tool detail'}))
}}
