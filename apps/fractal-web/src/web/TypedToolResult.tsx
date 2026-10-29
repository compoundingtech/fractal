import * as stylex from '@stylexjs/stylex'
import { Markdown } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Markdown.tsx'
import type { WorkLogCall } from '@fractal/ui/assistant-ui/work-log'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { projectTypedToolResult, recordedToolEvidence, recordedWebHref } from './typedToolResult.ts'

export interface TypedToolResultProps { readonly call: WorkLogCall }

/** App-owned result semantics; the inspector owns invocation input and lifecycle chrome. */
export const TypedToolResult = ({ call }: TypedToolResultProps) => {
  const result = projectTypedToolResult(call)
  const raw = recordedToolEvidence(call)
  return <section aria-label="Recorded tool result" data-typed-tool-result={result.kind} {...stylex.props(styles.root)}>
    {result.skipped && <p role="status" {...stylex.props(styles.note)}>Skipped</p>}
    {(result.kind === 'search' || result.kind === 'web') && <dl {...stylex.props(styles.facts)}>
      {result.query !== undefined && <><dt>Query</dt><dd {...stylex.props(styles.value)}><pre {...stylex.props(styles.text)}>{result.query}</pre></dd></>}
      {result.kind === 'search' && result.path !== undefined && <><dt>Path</dt><dd {...stylex.props(styles.value)}><pre {...stylex.props(styles.text)}>{result.path}</pre></dd></>}
      {result.kind === 'web' && result.url !== undefined && <><dt>URL</dt><dd {...stylex.props(styles.value)}>{recordedWebHref(result.url) === undefined ? result.url : <a href={recordedWebHref(result.url)} target="_blank" rel="noreferrer">{result.url}</a>}</dd></>}
    </dl>}
    {result.kind === 'search' && <>
      {result.files.length > 0 && <section aria-label="Matched files"><h3 {...stylex.props(styles.label)}>Files</h3><ul {...stylex.props(styles.list)}>{result.files.map((file, index) => <li key={index}><code>{file}</code></li>)}</ul></section>}
      {result.matches.length > 0 && <section aria-label="Search matches"><h3 {...stylex.props(styles.label)}>Matches</h3><ul {...stylex.props(styles.list)}>{result.matches.map((match, index) => <li key={index}>{match.path !== undefined && <code>{match.path}</code>}{match.line !== undefined && <span> · line {match.line}</span>}{match.text !== undefined && <pre {...stylex.props(styles.text)}>{match.text}</pre>}</li>)}</ul></section>}
    </>}
    {result.kind === 'web' && result.sources.length > 0 && <section aria-label="Recorded web sources"><h3 {...stylex.props(styles.label)}>Sources</h3><ul {...stylex.props(styles.list)}>{result.sources.map((source, index) => <li key={index}>
      {source.title !== undefined && <strong>{source.title}</strong>}
      {source.url !== undefined && <div>{recordedWebHref(source.url) === undefined ? source.url : <a href={recordedWebHref(source.url)} target="_blank" rel="noreferrer">{source.url}</a>}</div>}
      {source.snippet !== undefined && <p {...stylex.props(styles.note)}>{source.snippet}</p>}
    </li>)}</ul></section>}
    {result.kind === 'subagent' && result.tasks.length > 0 && <section aria-label="Recorded tasks"><h3 {...stylex.props(styles.label)}>Tasks</h3><ul {...stylex.props(styles.list)}>{result.tasks.map((task, index) => <li key={index}>{task.requested && <p role="status" {...stylex.props(styles.note)}>Requested</p>}{task.name !== undefined && <strong>{task.name}</strong>}{task.agent !== undefined && task.agent !== task.name && <p {...stylex.props(styles.note)}>Agent type: {task.agent}</p>}{task.state !== undefined && <p {...stylex.props(styles.note)}>{task.state === 'pending' ? 'State at task launch' : 'Recorded state at tool return'}: {task.state}</p>}{task.ref !== undefined && <pre {...stylex.props(styles.text)}>{task.ref}</pre>}{task.sessionId !== undefined && <p {...stylex.props(styles.note)}>Session: {task.sessionId}</p>}{task.report !== undefined && <section aria-label="Recorded task report"><Markdown text={task.report} /></section>}</li>)}</ul></section>}
    {result.kind === 'wait' && <dl {...stylex.props(styles.facts)}>
      {result.awaited.length > 0 && <><dt>Awaited</dt><dd {...stylex.props(styles.value)}>{result.awaited.map((ref, index) => <pre key={index} {...stylex.props(styles.text)}>{ref}</pre>)}</dd></>}
      {result.arrived.length > 0 && <><dt>Arrived</dt><dd {...stylex.props(styles.value)}>{result.arrived.map((ref, index) => <pre key={index} {...stylex.props(styles.text)}>{ref}</pre>)}</dd></>}
      {result.senders.length > 0 && <><dt>From</dt><dd {...stylex.props(styles.value)}>{result.senders.join(', ')}</dd></>}
      {result.stillRunning.length > 0 && <><dt>Still running</dt><dd {...stylex.props(styles.value)}>{result.stillRunning.map((ref, index) => <pre key={index} {...stylex.props(styles.text)}>{ref}</pre>)}</dd></>}
      {result.running && <><dt>Status</dt><dd {...stylex.props(styles.value)}>Running</dd></>}
      {result.interrupted && <><dt>Status</dt><dd {...stylex.props(styles.value)}>Interrupted</dd></>}
      {result.completed && !result.running && !result.interrupted && <><dt>Status</dt><dd {...stylex.props(styles.value)}>Completed</dd></>}
    </dl>}
    {result.kind !== 'unknown' && result.body !== undefined && <section aria-label="Recorded result body" {...stylex.props(styles.body)}>{result.kind === 'web' || result.kind === 'subagent' ? <Markdown text={result.body} /> : <pre {...stylex.props(styles.text)}>{result.body}</pre>}</section>}
    {raw !== undefined && <details {...stylex.props(styles.raw)}><summary>Raw evidence</summary><pre {...stylex.props(styles.text)}>{raw}</pre></details>}
  </section>
}
const styles = stylex.create({
  root: { minWidth: 0, display: 'flex', flexDirection: 'column', gap: s.md, color: c.fg, fontSize: t.metaSize, overflowWrap: 'anywhere' },
  facts: { display: 'grid', gridTemplateColumns: 'max-content minmax(0, 1fr)', gap: s.sm, margin: 0 },
  value: { margin: 0, minWidth: 0 },
  text: { margin: 0, maxHeight: 320, maxWidth: '100%', overflow: 'auto', whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontFamily: t.fontMono, fontSize: t.denseSize, lineHeight: 1.6 },
  body: { minWidth: 0, maxHeight: 480, overflow: 'auto' },
  list: { margin: 0, paddingInlineStart: s.lg, display: 'flex', flexDirection: 'column', gap: s.sm },
  label: { marginBlock: s.sm, fontSize: t.metaSize, color: c.fgMuted },
  note: { marginBlock: s.xs, color: c.fgMuted, lineHeight: 1.5 },
  raw: { color: c.fgMuted, minWidth: 0 },
})
