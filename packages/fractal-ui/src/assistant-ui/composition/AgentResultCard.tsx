import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { Markdown } from './Markdown'
import { formatWorkDuration } from '../taste/work-log'
import type { AgentResultData, ResultFact } from './delivery-model'
import { deliveryStyles as styles } from './delivery-styles'
import { DeliveryActionButton } from './DeliveryActionButton'
import { ResultJsonTree } from './ResultJsonTree'
export function agentResultNeedsExpansion(result: AgentResultData): boolean {
  return result.state._tag === 'Failed' || result.state._tag === 'Cancelled' || result.state._tag === 'Aborted'
}
function factLabel(fact: ResultFact): string {
  switch (fact._tag) {
    case 'Commit': return `commit ${fact.value}`
    case 'Branch': return fact.value
    case 'Pushed': return fact.value ? 'Pushed' : 'Not pushed'
    case 'Checks': return `checks ${fact.value}`
    case 'ExitCode': return `rc ${fact.value}`
    case 'Count': return `${fact.value} ${fact.label}`
  }
}
export function AgentResultCard(props: { readonly result: AgentResultData; readonly expanded?: boolean; readonly compact?: boolean }) {
  // A new harness state starts a new disclosure decision; ordinary toggles retain the same control.
  return <ResultPresentation key={`${props.result.id}:${props.result.state._tag}`} {...props} />
}
function ResultPresentation({ result, expanded = false, compact = false }: { readonly result: AgentResultData; readonly expanded?: boolean; readonly compact?: boolean }) {
  const [open, setOpen] = React.useState(() => agentResultNeedsExpansion(result))
  const [rawOpen, setRawOpen] = React.useState(false)
  const visible = expanded || open
  const state = result.state
  const excerpt = state._tag === 'Failed' ? state.error : state._tag === 'Cancelled' || state._tag === 'Aborted' ? state.reason : result.excerpt
  const lifecycle = result.lifecycle._tag === 'Stopped' ? `Stopped${result.lifecycle.resumable ? ' · resumable' : ''}` : result.lifecycle._tag === 'Isolated' ? 'Ran isolated' : result.lifecycle._tag === 'Submitted' ? 'Result submitted' : result.lifecycle._tag
  return <article data-testid="agent-result" data-result-state={state._tag} {...stylex.props((!compact || visible) && styles.card, styles.stack)}>
    <div {...stylex.props(styles.row)}><Button aria-label={visible ? 'Hide result' : 'Show result'} aria-expanded={visible} aria-disabled={expanded} onPress={expanded ? undefined : () => setOpen(value => !value)} {...stylex.props(styles.button, styles.summary)}><span aria-hidden="true">◈</span><strong>{result.name}</strong>{result.agentKind && <span {...stylex.props(styles.muted)}>{result.agentKind}</span>}{state._tag !== 'Completed' && <span {...stylex.props(styles.chip, state._tag === 'Failed' && styles.danger)}>{state._tag}{state._tag === 'Failed' && state.exitCode !== undefined ? ` · exit ${state.exitCode}` : ''}</span>}{result.schema._tag === 'Invalid' && <span title={result.schema.reason} {...stylex.props(styles.chip, styles.danger)}>Schema invalid</span>}{result.durationMs !== undefined && <span {...stylex.props(styles.muted)}>{formatWorkDuration(result.durationMs) || '<1s'}</span>}{result.time && <time {...stylex.props(styles.muted)}>{result.time}</time>}<span aria-hidden="true">{visible ? '⌄' : '›'}</span></Button></div>
    {(!compact || visible) && <>{excerpt && <p {...stylex.props(styles.excerpt)}>{excerpt}</p>}{result.facts.length > 0 && <div {...stylex.props(styles.row)}>{result.facts.slice(0, 5).map((fact, index) => <span key={index} {...stylex.props(styles.chip)}>{factLabel(fact)}</span>)}{result.facts.length > 5 && <span {...stylex.props(styles.muted)}>+{result.facts.length - 5}</span>}</div>}</>}
    {visible && <div {...stylex.props(styles.stack)}><Markdown text={result.report.text} />{result.report._tag === 'Preview' && <div {...stylex.props(styles.row)}><span {...stylex.props(styles.muted)}>Result preview</span><DeliveryActionButton label="Full output" action={result.report.fullOutput} /></div>}<div {...stylex.props(styles.row)}><Button aria-expanded={rawOpen} onPress={() => setRawOpen(value => !value)} {...stylex.props(styles.button)}>{rawOpen ? 'Hide raw JSON' : 'Show raw JSON'}</Button><Button onPress={() => { void navigator.clipboard.writeText(JSON.stringify(result.raw, null, 2)) }} {...stylex.props(styles.button)}>Copy JSON</Button><span {...stylex.props(styles.muted)}>{result.report.text.split('\n').length} lines · {new TextEncoder().encode(result.report.text).length} bytes</span></div>{rawOpen && <div aria-label="Raw result JSON" {...stylex.props(styles.tree)}><ResultJsonTree label="result" value={result.raw} /></div>}</div>}
    {(!compact || visible) && <footer {...stylex.props(styles.footer)}><span {...stylex.props(styles.muted)}>{lifecycle}</span><DeliveryActionButton label="Open thread" action={result.openThread} /><DeliveryActionButton label="Message" action={{ _tag: 'Unavailable', reason: result.messageReason }} /><DeliveryActionButton label="Resume" action={{ _tag: 'Unavailable', reason: result.resumeReason }} /></footer>}
  </article>
}
