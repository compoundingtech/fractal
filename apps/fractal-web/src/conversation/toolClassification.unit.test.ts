import { describe, expect, it } from 'vitest'
import { decodeConversationChunk } from '@fractal/sdk/effect'
import { LiveTimeline } from './fromTimeline.ts'
import type { ToolCallItem } from './model.ts'
import { toolKindFor } from './toolClassification.ts'
import { semanticToolFor } from './semanticToolModel.ts'
import { prepareTranscriptTurns } from '../web/conversationTranscript.ts'
const at = '2026-01-15T12:00:00Z'
const call = (name: string, input: unknown = {}, extra: Partial<ToolCallItem> = {}): ToolCallItem => ({ _tag: 'ToolCall', id: 'example/call', callId: 'example/call', name, input, status: 'success', callSeen: true, at, ...extra })

describe('typed semantic tool classification', () => {
  it.each([['wait','wait'],['write_stdin','terminal-input'],['TaskCreate','todo'],['TaskUpdate','todo'],['SendMessage','message'],['request_user_input','decision'],['Monitor','schedule'],['exec_command','run'],['grep','search'],['web_search','web'],['task','subagent'],['tools.exec_command','run']])('classifies exact operation %s as %s', (name, kind) => expect(toolKindFor(call(name))).toBe(kind))
  it('prefers view.type to misleading tool name; unknown never becomes read', () => {
    expect(toolKindFor(call('TaskCreate', {}, { view: { type: 'wait' } }))).toBe('wait')
    expect(toolKindFor(call('read', {}, { view: { type: 'future-operation' } }))).toBe('unknown')
    expect(toolKindFor('agent-plan-reader')).toBe('unknown')
    expect(toolKindFor('task_monitor_report')).toBe('unknown')
  })
  it('classifies agent URI writes only, never files containing agent', () => {
    expect(toolKindFor(call('write', { path: 'agent://Builder', content: 'Ready.' }))).toBe('message')
    expect(toolKindFor(call('write', { path: 'notes/agent-plan.md' }))).toBe('edit')
    expect(toolKindFor(call('write', { path: 'notes/result.md' }, { view: { type: 'write', path: 'agent://Builder' } }))).toBe('message')
  })
  it('projects one grouped outgoing card with native receipts and body', () => {
    const message = call('write', { path: 'agent://all', content: 'Ready for review.' }, { result: { content: 'sent', isError: false, at, metadata: { message: { to: ['Builder','Reviewer'], receipts: [{to:'Builder',outcome:'injected'},{to:'Reviewer',outcome:'queued'}] } } } })
    expect(semanticToolFor(message)).toMatchObject({type:'message',recipients:['Builder','Reviewer'],broadcast:true,body:'Ready for review.',receipts:[{to:'Builder',outcome:'injected'},{to:'Reviewer',outcome:'queued'}]})
  })
  it('keeps wait arrival facts distinct from file reads', () => {
    const wait = call('wait', {jobs:['bg_1']}, {result:{content:'arrived',isError:false,at,metadata:{jobs:[{jobId:'bg_1',status:'completed'}],waited:12}}})
    expect(semanticToolFor(wait)).toEqual({type:'wait',awaited:['bg_1'],arrived:['bg_1'],stillRunning:[],senders:[],interrupted:false,running:false,waited:12})
    expect(semanticToolFor(call('read',{path:'src/rows.ts'}))).toBeUndefined()
  })
  it('renders synthetic interrupt_skipped as neutral skipped work', () => {
    const timeline = new LiveTimeline()
    timeline.apply(decodeConversationChunk({kind:'conversation',collection:'conversation',id:'follow/example',session_id:'session/example',replace:true,has_more:false,items:[
      {id:'timeline-entry/example/1',sequence:1,revision:1,final:true,role:'tool',timestamp:at,type:'tool_call',body:{call_id:'call/example',name:'bash',arguments:{command:'printf ready'}}},
      {id:'timeline-entry/example/2',sequence:2,revision:1,final:true,role:'tool',timestamp:at,type:'tool_result',body:{call_id:'call/example',status:'error',media_type:'text/plain',content:'Interrupted before execution.',blocks:[{id:'block/result',kind:'tool_output',source_type:'omp',payload:{body_ref:true},metadata:{__synthetic:true,source:'interrupt_skipped',executed:false}}]}},
    ]}))
    const items = timeline.project().items
    expect(items[0]).toMatchObject({_tag:'ToolCall',status:'success',skipped:true,result:{isError:false}})
    const turns = prepareTranscriptTurns(items,{firstTurnComplete:true})
    expect(turns[0]?.work).toMatchObject({failed:false,calls:[{title:'Skipped',skipped:true}]})
  })
})
