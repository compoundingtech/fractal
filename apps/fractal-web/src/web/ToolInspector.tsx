import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { FocusScope } from 'react-aria'
import { EmbraceDiffPreview, HighlightedSource, toolDiffs } from '@smalltalk/fractal-ui/assistant-ui'
import { readableCommand, workLogTurnFromItems, type WorkLogCall } from '@smalltalk/fractal-ui/assistant-ui/work-log'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { toolString } from '../../../../packages/fractal-ui/src/assistant-ui/embrace-tool-preview.ts'
import { useConversation } from '../data/react.tsx'
import { toolKindFor } from './conversationTranscript.ts'
import { RecordedToolText } from './RecordedToolText.tsx'
import { TypedToolResult } from './TypedToolResult.tsx'
import { recordedTaskRequests } from './typedToolResult.ts'
import { TaskInvocation } from './TaskInvocation.tsx'
import { outgoingMessageFor } from '../conversation/semanticToolModel.ts'
import { SemanticToolDetail } from './semanticTool.tsx'

export interface ToolInspectorProps {
  readonly call: WorkLogCall
  readonly width?: number
  readonly onClose: () => void
}

/** An opened call follows its real source item. A paged-out call stays an explicitly captured
 * snapshot, rather than pretending disappearance means completion or losing its inspection. */
export const LiveToolInspector = ({ agentRef, call, ...props }: ToolInspectorProps & { readonly agentRef: string }) => {
  const feed = useConversation(agentRef)
  const item = feed._tag === 'Observed' ? feed.value.items.find(item => item.id === call.id && item._tag === 'ToolCall') : undefined
  const current = item?._tag === 'ToolCall' ? workLogTurnFromItems([item], {
    kindFor: () => toolKindFor(item), running: item.status === 'running', failed: item.status === 'error', interrupted: item.status === 'interrupted',
  }).calls[0] : undefined
  return <ToolInspector {...props} call={current === undefined ? call : { ...current, title: call.title, summary: call.summary }} />
}

/** This component owns no drawer or transport: the workspace and future Details pane can host it. */
export const ToolInspector = (props: ToolInspectorProps) => <FocusScope key={props.call.id} autoFocus restoreFocus>
  <ToolInspectorContent {...props} onKeyDown={event => {
    if (event.key !== 'Escape' || event.defaultPrevented) return
    event.preventDefault()
    event.stopPropagation()
    props.onClose()
  }} />
</FocusScope>

/** The content-only surface lets story controls exercise missing focus ownership. Hosts use ToolInspector. */
export const ToolInspectorContent = ({ call, width, onClose, onKeyDown }: ToolInspectorProps & { readonly onKeyDown?: React.KeyboardEventHandler<HTMLElement> }) => {
  const command = call.kind === 'message' ? undefined : readableCommand(call)
  const start = Date.parse(call.startedAt)
  const end = call.endedAt === undefined ? NaN : Date.parse(call.endedAt)
  const duration = Number.isFinite(start) && Number.isFinite(end) && end >= start ? end - start : undefined
  const diffs = call.kind === 'change' && call.status === 'success' ? toolDiffs(call.input, call.output ?? call.detail) : []
  const recordedChange = call.kind === 'change' && call.status === 'success' ? toolString(call.input, ['content', 'patch', 'input']) : ''
  const source = call.detail
  const typed = call.kind === 'search' || call.kind === 'web' || call.kind === 'subagent' || call.kind === 'wait'
  const message = call.kind === 'message' ? outgoingMessageFor(call) : undefined
  const specialized = message !== undefined ? 'message' : typed ? call.kind : call.kind === 'run' ? 'run' : call.kind === 'read' && source !== undefined ? 'read' : diffs.length > 0 || recordedChange !== '' ? 'change' : 'raw'
  const output = source ?? (call.output === undefined ? undefined : typeof call.output === 'string' ? call.output : JSON.stringify(call.output, null, 2))
  const inputFields = typeof call.input === 'object' && call.input !== null && !Array.isArray(call.input) ? Object.entries(call.input) : undefined
  const taskRequests = call.kind === 'subagent' ? recordedTaskRequests(call.input) : undefined
  const taskNames = taskRequests?.flatMap(task => task.name === undefined ? [] : [task.name])
  const title = taskNames !== undefined && taskNames.length > 0 ? taskNames.join(', ') : call.title
  return <aside aria-label="Tool detail" data-tool-inspector={call.id} data-result-view={specialized} onKeyDown={onKeyDown} style={{ width }} {...stylex.props(styles.root)}>
    <header {...stylex.props(styles.header)}><div {...stylex.props(styles.heading)}><h2 {...stylex.props(styles.title)}>{title}</h2><p {...stylex.props(styles.note)}>Recorded tool invocation · loaded transcript</p></div><Button aria-label="Close tool detail" onPress={onClose} {...stylex.props(styles.close)}>×</Button></header>
    <div {...stylex.props(styles.body)}>
      <dl {...stylex.props(styles.facts)}><dt>Tool</dt><dd data-tool-name="" {...stylex.props(styles.factValue)}>{call.toolName ?? call.title}</dd><dt>Status</dt><dd data-tool-inspector-status="" {...stylex.props(styles.factValue)}>{{ running: 'Running', success: 'Succeeded', error: 'Failed', interrupted: 'Interrupted' }[call.status]}</dd><dt>Duration</dt><dd data-tool-duration="" {...stylex.props(styles.factValue)}>{duration === undefined ? 'Not reported' : duration < 1000 ? `${duration} ms` : `${(duration / 1000).toFixed(1)} s`}</dd><dt>Tool call ID</dt><dd data-tool-call-id="" {...stylex.props(styles.factValue)}>{call.callId ?? 'Not recorded'}</dd><dt>Native entry ID</dt><dd data-native-entry-id="" {...stylex.props(styles.factValue)}>{call.id}</dd></dl>
      {command !== undefined && <section aria-label="Command"><h3 {...stylex.props(styles.label)}>Command</h3><pre data-testid="tool-inspector-command" {...stylex.props(styles.code)}><code><HighlightedSource code={command} language="bash" /></code></pre></section>}
      <section aria-label="Tool input" data-tool-input=""><h3 {...stylex.props(styles.label)}>Input</h3>{message !== undefined ? <details><summary>Raw invocation input</summary><RecordedToolText output={false} text={typeof call.input === 'string' ? call.input : JSON.stringify(call.input, null, 2) ?? 'No invocation input available in loaded history.'} /></details> : taskRequests !== undefined ? <TaskInvocation tasks={taskRequests} input={call.input} /> : call.input === undefined ? <p {...stylex.props(styles.note)}>No invocation input available in loaded history.</p> : inputFields === undefined ? <RecordedToolText text={typeof call.input === 'string' ? call.input : JSON.stringify(call.input, null, 2)} output={false} /> : <dl {...stylex.props(styles.inputFields)}>{inputFields.map(([key, value]) => <div key={key}><dt {...stylex.props(styles.label)}>{specialized === 'change' && ['content', 'patch', 'input'].includes(key) && typeof value === 'string' ? 'Recorded write/edit source' : key}</dt><dd {...stylex.props(styles.factValue)}><RecordedToolText output={false} text={typeof value === 'string' ? value : JSON.stringify(value, null, 2)} language={typeof value === 'string' ? '' : 'json'} recordedChange={specialized === 'change' && ['content', 'patch', 'input'].includes(key) && typeof value === 'string'} /></dd></div>)}</dl>}</section>
      <section aria-label="Tool output"><h3 {...stylex.props(styles.label)}>{message !== undefined ? 'Recorded message' : typed ? 'Recorded result' : specialized === 'raw' ? 'Raw output · no specialized renderer' : specialized === 'run' ? 'Command output' : specialized === 'read' ? 'Read result' : 'Captured change'}</h3>
        {message !== undefined ? <SemanticToolDetail model={message} /> : typed ? <TypedToolResult call={call} /> : specialized === 'change' ? <><p {...stylex.props(styles.note)}>Recorded successful-tool input and result, not current repository state.</p>{diffs.length > 0 ? <EmbraceDiffPreview diffs={diffs} /> : <RecordedToolText text={output ?? 'No output recorded.'} />}</>
          : output === undefined ? <p role="status" {...stylex.props(styles.note)}>{call.status === 'running' ? 'No output received yet.' : 'No output recorded.'}</p>
          : <RecordedToolText text={output} markdown={specialized === 'read' && call.outputLanguage === 'markdown'} language={specialized === 'read' ? call.outputLanguage ?? '' : specialized === 'raw' && typeof call.output === 'object' ? 'json' : ''} />}
      </section>
      <details data-tool-raw="" {...stylex.props(styles.raw)}><summary>Raw recorded output</summary><RecordedToolText text={call.output === undefined ? source ?? 'Output not recorded.' : typeof call.output === 'string' ? call.output : JSON.stringify(call.output, null, 2)} /></details>
    </div>
  </aside>
}
const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', flexShrink: 0, minWidth: 0, minHeight: 0, overflow: 'hidden', color: c.fg, backgroundColor: c.canvas, borderLeftWidth: 1, borderLeftStyle: 'solid', borderLeftColor: c.border },
  header: { display: 'flex', alignItems: 'flex-start', gap: s.md, padding: s.lg, flexShrink: 0, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: c.border },
  heading: { flex: 1, minWidth: 0 },
  title: { margin: 0, fontSize: t.metaSize, lineHeight: 1.5, overflowWrap: 'anywhere' },
  note: { marginBlock: s.xs, color: c.fgMuted, fontSize: t.metaSize, lineHeight: 1.5 },
  close: { backgroundColor: 'transparent', color: c.fg, borderWidth: 0, cursor: 'pointer', minWidth: 28, minHeight: 28, fontSize: 18 },
  body: { minHeight: 0, overflowY: 'auto', padding: s.lg, display: 'flex', flexDirection: 'column', gap: s.lg },
  facts: { display: 'grid', gridTemplateColumns: 'max-content 1fr', gap: s.sm, margin: 0, fontSize: t.metaSize, lineHeight: 1.5 },
  factValue: { margin: 0, overflowWrap: 'anywhere' },
  inputFields: { margin: 0, display: 'flex', flexDirection: 'column', gap: s.sm },
  label: { marginBlock: s.sm, fontSize: t.metaSize, color: c.fgMuted, fontWeight: 600 },
  code: { margin: 0, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontFamily: t.fontMono, fontSize: t.denseSize, lineHeight: 1.6, color: c.fg },
  raw: { fontSize: t.metaSize, color: c.fgMuted },
})
