import { describe, expect, it } from 'vitest'
import { decodeConversationChunk } from '@fractal/sdk/effect'
import { LiveTimeline } from './fromTimeline.ts'
import type { ToolCallItem } from './model.ts'
import { toolKindFor } from './toolClassification.ts'
import { semanticToolFor } from './semanticToolModel.ts'
const at = '2026-01-15T12:00:00Z'
const call = (name: string, input: unknown = {}, extra: Partial<ToolCallItem> = {}): ToolCallItem => ({ _tag: 'ToolCall', id: 'example/call', callId: 'example/call', name, input, status: 'success', callSeen: true, at, ...extra })

describe("recorded CLI message sends", () => {
  it('projects the recorded bash send shape without replacing its shell evidence', () => {
    const body = "Please inspect the recorded review. It includes ('quoted prose'), a path /workspace/example and ordinary punctuation."
    const command = `st conversations send agent/example/Reviewer --from agent/example/Builder --idempotency-key example-send-1 --subject "Recorded review" --body "${body}" </dev/null 2>&1 | tail -1`
    const timeline = new LiveTimeline()
    timeline.apply(decodeConversationChunk({kind:'conversation',collection:'conversation',id:'follow/recorded-send',session_id:'session/example',replace:true,has_more:false,items:[
      {id:'timeline-entry/example/send',sequence:1,revision:1,final:true,role:'tool',timestamp:at,type:'tool_call',body:{call_id:'call/recorded-send',name:'bash',arguments:{command,i:'send to reviewer',timeout:90}}},
    ]}))
    const message = timeline.project().items[0]
    expect(message?._tag).toBe('ToolCall')
    if (message?._tag !== 'ToolCall') return
    expect(toolKindFor(message)).toBe('message')
    expect(semanticToolFor(message)).toEqual({type:'message',recipients:['agent/example/Reviewer'],broadcast:false,body,subject:'Recorded review',receipts:[]})
    expect(message.input).toEqual({command,i:'send to reviewer',timeout:90})
    expect(message.status).toBe('running')
    expect(message.result).toBeUndefined()
  })
  it.each([
    ['run', 'st conversations send agent/example/Reviewer --from agent/example/Builder -m "Ready for review."', 'Ready for review.'],
    ['bash', "st --json --endpoint=http://localhost:9999 conversations send --subject='Review note' --as=agent/example/Builder --body='Ready for review.' agent/example/Reviewer", 'Ready for review.'],
    ['exec_command', String.raw`st conversations send agent/example/Reviewer \
      --from agent/example/Builder --body 'Ready for review.' --attach /workspace/example.png --tags review,ready`, 'Ready for review.'],
    ['Bash', String.raw`st conversations send agent/example/Reviewer --from agent/example/Builder --body 'Ready for '\''review.'`, "Ready for 'review."],
    ['bash', String.raw`st conversations send agent/example/Reviewer --from agent/example/Builder --body "A literal \$5 and \"quoted\" prose."`, 'A literal $5 and "quoted" prose.'],
    ['bash', "st conversations send agent/example/Reviewer --from agent/example/Builder --body 'Literal $BODY; `words`, $(not executed).'", 'Literal $BODY; `words`, $(not executed).'],
  ])('adapts the literal send grammar for %s', (name, command, body) => {
    const message = call(name, { command })
    expect(toolKindFor(message)).toBe('message')
    expect(semanticToolFor(message)).toMatchObject({type:'message',recipients:['agent/example/Reviewer'],body})
  })
  it.each([
    '| cat',
    '> receipt.txt',
    '</dev/null 2>&1 | tail -1',
    '2>>errors.txt | head -n 3 | tee receipt.txt',
    '>receipt.txt 2>&1',
    '2>&1>receipt.txt',
    '| tail -n 1 >"receipt with spaces.txt"',
  ])('keeps a literal sender with output plumbing as message: %s', suffix => {
    const command = `st conversations send agent/example/Reviewer --from agent/example/Builder --body "Ready." ${suffix}`
    const message = call('bash', {command})
    expect(toolKindFor(message)).toBe('message')
    expect(semanticToolFor(message)).toEqual({type:'message',recipients:['agent/example/Reviewer'],broadcast:false,body:'Ready.',receipts:[]})
    expect(message.input).toEqual({command})
  })
  it('prefers exact native command facts and message fields to legacy input', () => {
    const input = {command:'printf unrelated'}
    const argv = ['st','conversations','send','agent/example/Reviewer','--from','agent/example/Builder','--body','Literal $body; not a shell program.']
    const message = call('unknown-native-tool', input, {view:{type:'command',argv}})
    expect(toolKindFor(message)).toBe('message')
    expect(semanticToolFor(message)).toMatchObject({type:'message',recipients:['agent/example/Reviewer'],body:'Literal $body; not a shell program.'})
    const native = call('bash', {command:argv}, {result:{content:'recorded receipt',isError:false,at,view:{type:'agent_message_sent',to:['agent/example/Native'],body:'Native words.'},metadata:{message:{to:['agent/example/Confirmed'],body:'Confirmed words.',receipts:[{to:'agent/example/Confirmed',outcome:'queued'}]}}}})
    expect(semanticToolFor(native)).toEqual({type:'message',recipients:['agent/example/Confirmed'],broadcast:false,body:'Confirmed words.',receipts:[{to:'agent/example/Confirmed',outcome:'queued'}]})
    expect(toolKindFor(call('bash', {command:argv}, {view:{type:'bash',command:'printf unrelated'}}))).toBe('run')
    expect(toolKindFor(call('bash', {command:argv}, {view:{type:'read'}}))).toBe('read')
  })
  it('keeps a failed send truthful and accepts structured executable arguments', () => {
    const message = call('run', {executable:'st',argv:['conversations','send','agent/example/Reviewer','--from','agent/example/Builder','--body','Please review.']}, {status:'error',result:{content:'Permission denied.',isError:true,at}})
    expect(toolKindFor(message)).toBe('message')
    expect(semanticToolFor(message)).toEqual({type:'message',recipients:['agent/example/Reviewer'],broadcast:false,body:'Please review.',receipts:[]})
    expect(message.status).toBe('error')
    expect(message.result?.isError).toBe(true)
  })
  it.each([
    'echo st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready',
    '# st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready',
    'printf "%s" "st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready"',
    'bash -c "st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready"',
    'st conversations reply agent/example/Reviewer --from agent/example/Builder --body Ready',
    'st conversations status agent/example/Reviewer',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body "$BODY"',
    'st conversations send "$RECIPIENT" --from agent/example/Builder --body Ready',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body `cat note.txt`',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body "$(cat note.txt)"',
    'st conversations send agent/example/* --from agent/example/Builder --body Ready',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready; printf done',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready && printf done',
    'printf "%s" "st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready" | tail -1',
    'cat note.txt | st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready | bash',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready || printf failed',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready > "$RECEIPT"',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready | tail "$LIMIT"',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready >',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready 2>&',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready |',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready <<EOF',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready # mention',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready\nprintf done',
    'BODY=Ready st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready --print-kdl',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready --unknown',
    'st conversations send agent/example/Reviewer --from agent/example/Builder',
    'st conversations send agent/example/Reviewer --body Ready',
    'st conversations send --from agent/example/Builder --body Ready',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body --subject Ready',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body Ready --body Other',
    'st conversations send agent/example/Reviewer --from agent/example/Builder --body "unterminated',
  ])('leaves nonliteral or unrelated shell source as run: %s', command => {
    const tool = call('bash', {command})
    expect(toolKindFor(tool)).toBe('run')
    expect(semanticToolFor(tool)).toBeUndefined()
  })
})
