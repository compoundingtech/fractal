/** A GitHub Actions run in the pull request's shell: header, its pull requests as links, and a Jobs subtab. */
import * as stylex from '@stylexjs/stylex'

import { checkTone, checkWord, durationSeconds, formatDurationShort } from '../model.ts'
import type { GithubRef } from '../refs.ts'
import { githubUrlOf, labelOf, refOf } from '../refs.ts'
import type { GithubRead, GithubRun } from '../schema.ts'
import { CheckList, checkCountsLine } from './Checks.tsx'
import { staleWord, tabLabel } from './detail-model.ts'
import { DetailFrame, DetailHeader, DetailLoading, DetailTabs, detailStyles as d, LinkedSection, ReadProblem, type LinkedThing } from './DetailShell.tsx'

export interface RunDetailProps {
  readonly target: Extract<GithubRef, { kind: 'run' }>
  /** `undefined` while loading. */
  readonly run: GithubRead<GithubRun> | undefined
  readonly linked?: readonly LinkedThing[] | undefined
  readonly now: number
  readonly onOpen?: ((ref: string) => void) | undefined
}

export function RunDetail({ target, run, linked = [], now, onOpen }: RunDetailProps) {
  const label = labelOf(target)
  if (run === undefined) return <DetailFrame label={`Run ${label}`}><DetailLoading what="run" target={label} url={githubUrlOf(target)} /></DetailFrame>
  if (run.status !== 'found') return <DetailFrame label={`Run ${label}`}><ReadProblem read={run} target={target} onOpen={onOpen} /></DetailFrame>
  const item = run.item
  const seconds = durationSeconds(item.started_at, item.completed_at, now)
  const pullRequests: readonly LinkedThing[] = item.pull_requests.map(number => ({ ref: refOf({ kind: 'pull-request', owner: item.owner, repo: item.repo, number }), label: `#${number}`, kind: 'pull-request' }))
  const facts = [
    item.head_branch === null ? undefined : <span key="branch" {...stylex.props(d.mono)}>{item.head_branch}</span>,
    <span key="sha" {...stylex.props(d.mono)}>{item.head_sha.slice(0, 7)}</span>,
    item.event,
    item.run_attempt > 1 ? `attempt ${item.run_attempt}` : undefined,
    seconds === undefined ? undefined : formatDurationShort(seconds),
  ].filter(fact => fact !== undefined)
  return <DetailFrame label={`Run ${label}`}>
    <DetailHeader title={item.name} handle={`run ${item.id}`} state={checkWord(item)} tone={checkTone(item)} url={item.url} stale={staleWord(run, now)} facts={facts} />
    <LinkedSection items={[...linked, ...pullRequests]} onOpen={onOpen} />
    <DetailTabs label="Run sections" selected="jobs" onSelect={() => {}} tabs={[{
      id: 'jobs',
      label: tabLabel('Jobs', item.jobs.length),
      panel: <div {...stylex.props(d.stack)}>
        <p data-check-counts {...stylex.props(d.summaryLine)}>{checkCountsLine(item.jobs)}</p>
        <CheckList label="Jobs" runs={item.jobs} now={now} />
      </div>,
    }]} />
  </DetailFrame>
}
