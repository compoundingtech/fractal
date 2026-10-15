// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import type { ToolCallItem } from '../conversation/model.ts'
import { prepareTranscriptTurns } from './conversationTranscript.ts'
import { ToolInspector } from './ToolInspector.tsx'
vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (values: unknown) => values, createTheme: () => ({}), keyframes: () => '', props: () => ({}) }))
const at='2026-01-15T12:00:00Z'
const command='st conversations send agent/example/Reviewer --from agent/example/Builder --subject "Recorded review" --body "Inspect the **recorded change**." </dev/null 2>&1 | tail -1'
const item: ToolCallItem={_tag:'ToolCall',id:'example/send',callId:'call/send',name:'bash',input:{command},status:'success',callSeen:true,at,result:{content:'Message sent.',isError:false,at}}
it('shows destination, subject and Markdown in the message inspector; shell evidence is explicit raw input',async()=>{
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true)
  const container=document.createElement('div'),root=createRoot(container)
  try{
    const call=prepareTranscriptTurns([item],{firstTurnComplete:true})[0]!.work.calls[0]!
    expect(call.kind).toBe('message')
    await act(async()=>root.render(<ToolInspector call={call} onClose={()=>{}} />))
    expect(container.querySelector('[data-result-view="message"]')).not.toBeNull()
    expect(container.querySelector('[data-testid="tool-inspector-command"]')).toBeNull()
    expect(container.querySelector('[data-message-subject]')?.textContent).toBe('Recorded review')
    expect(container.querySelector('[data-testid="markdown"] strong')?.textContent).toBe('recorded change')
    const primary=container.cloneNode(true) as HTMLElement
    for(const details of primary.querySelectorAll('details'))details.remove()
    expect(primary.textContent).toContain('agent/example/Reviewer')
    expect(primary.textContent).not.toContain('st conversations send')
    expect(container.querySelector('[data-tool-input] details')?.getAttribute('open')).toBeNull()
    expect(container.querySelector('[data-tool-input] details')?.textContent).toContain(JSON.stringify({command}, null, 2))
    const control={...item,id:'example/mention',input:{command:'printf "st conversations send"'}}
    const run=prepareTranscriptTurns([control],{firstTurnComplete:true})[0]!.work.calls[0]!
    expect(run.kind).toBe('run')
    await act(async()=>root.render(<ToolInspector call={run} onClose={()=>{}} />))
    expect(container.querySelector('[data-testid="outgoing-message-card"]')).toBeNull()
  }finally{await act(async()=>root.unmount());vi.unstubAllGlobals()}
})
