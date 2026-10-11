import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { formatWorkDuration } from '../taste/work-log'
import type { BackgroundJobData } from './delivery-model'
import { deliveryStyles as styles } from './delivery-styles'
import { TerminalTail } from './TerminalTail'
export function backgroundJobNeedsExpansion(job: BackgroundJobData): boolean {
  return job.state._tag === 'Cancelled' || job.state._tag === 'Aborted' || job.state._tag === 'Exited' && (job.state.exit._tag === 'NotReported' || job.state.exit.code !== 0)
}
/** Shares WorkLogV1's row geometry, without inferring a run failure from terminal text. */
export function BackgroundJobRow(props: { readonly job: BackgroundJobData; readonly expanded?: boolean }) {
  return <JobPresentation key={props.job.id} {...props} />
}
function JobPresentation({ job, expanded = false }: { readonly job: BackgroundJobData; readonly expanded?: boolean }) {
  const state = job.state
  const stateKey = state._tag === 'Exited' ? `${state._tag}:${state.exit._tag}:${state.exit._tag === 'Reported' ? state.exit.code : ''}` : state._tag
  const [disclosure, setDisclosure] = React.useState(() => ({ stateKey, open: backgroundJobNeedsExpansion(job) }))
  if (disclosure.stateKey !== stateKey) {
    setDisclosure({ stateKey, open: disclosure.open || backgroundJobNeedsExpansion(job) })
  }
  const visible = expanded || disclosure.open
  const exit = job.state._tag === 'Exited' && job.state.exit._tag === 'Reported' ? job.state.exit.code : undefined
  return <section data-testid="background-job" data-job-id={job.id} data-exit={exit} {...stylex.props(styles.stack)}>
    <div {...stylex.props(styles.row)}><span aria-hidden="true">{job.kind === 'service' ? '◉' : '$'}</span><Button aria-label={`Output for ${job.id}`} aria-expanded={visible} aria-disabled={expanded} onPress={expanded ? undefined : () => setDisclosure(current => ({ ...current, open: !current.open }))} {...stylex.props(styles.button, styles.summary, styles.label)}><code title={job.command} {...stylex.props(styles.mono, styles.label)}>{job.command}</code><span aria-hidden="true">{visible ? '⌄' : '›'}</span></Button>{exit !== undefined && <span data-testid="exit-chip" {...stylex.props(styles.chip, exit !== 0 && styles.danger)}>exit {exit}</span>}{job.state._tag === 'Ready' && <span {...stylex.props(styles.chip)}>Ready</span>}{job.state._tag === 'Monitor' && <span {...stylex.props(styles.muted)}>{job.state.event}</span>}{(job.state._tag === 'Cancelled' || job.state._tag === 'Aborted') && <span {...stylex.props(styles.chip)}>{job.state._tag}</span>}{job.durationMs !== undefined && <span {...stylex.props(styles.muted)}>{formatWorkDuration(job.durationMs) || '<1s'}</span>}<span {...stylex.props(styles.muted)}>{job.id}</span></div>
    {(job.state._tag === 'Cancelled' || job.state._tag === 'Aborted') && <p {...stylex.props(styles.excerpt)}>{job.state.reason}</p>}
    {visible && <TerminalTail output={job.output} />}
    {job.startedBy && <Button onPress={job.startedBy.onPress} {...stylex.props(styles.button)}>Started by {job.startedBy.label}</Button>}
  </section>
}
