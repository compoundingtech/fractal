import * as stylex from '@stylexjs/stylex'
import { Markdown } from '@smalltalk/fractal-ui/assistant-ui'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { RecordedTaskRequest } from './typedToolResult.ts'

export const TaskInvocation = ({ tasks, input }: { readonly tasks: readonly RecordedTaskRequest[]; readonly input: unknown }) => <div data-task-invocation="" {...stylex.props(styles.root)}>{tasks.map((task, index) => <section key={index} aria-label={task.name ?? `Requested task ${index + 1}`}><dl {...stylex.props(styles.facts)}>{task.name !== undefined && <><dt>Name</dt><dd>{task.name}</dd></>}{task.agent !== undefined && <><dt>Agent type</dt><dd>{task.agent}</dd></>}</dl><h4 {...stylex.props(styles.label)}>Task</h4><Markdown text={task.task} /></section>)}<details><summary>Raw invocation input</summary><pre {...stylex.props(styles.raw)}>{JSON.stringify(input, null, 2)}</pre></details></div>
const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', gap: s.md, minWidth: 0, fontSize: t.metaSize },
  facts: { display: 'grid', gridTemplateColumns: 'max-content minmax(0, 1fr)', gap: s.sm, margin: 0 },
  label: { color: c.fgMuted, fontSize: t.metaSize, marginBlock: s.sm },
  raw: { whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontSize: t.denseSize },
})
