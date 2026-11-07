/** Check runs (a PR's Checks subtab) and workflow jobs (a run's Jobs subtab): GitHub's words, links out for logs. */
import * as stylex from '@stylexjs/stylex'

import { checkCounts, checkTone, checkWord, durationSeconds, formatDurationShort } from '../model.ts'
import type { CheckStatus } from '../schema.ts'
import { detailStyles as d, ExternalLink, toneStyle } from './DetailShell.tsx'

export interface CheckRow {
  readonly name: string
  readonly workflow?: string | null
  readonly status: CheckStatus
  readonly conclusion: string | null
  readonly started_at: string | null
  readonly completed_at: string | null
  readonly url: string
  readonly summary?: string | null
}

/** `1 passed · 1 failed · 2 pending · 1 skipped or other`, zero counts left out. */
export const checkCountsLine = (runs: readonly CheckRow[]): string => {
  const counts = checkCounts(runs)
  return [
    counts.failure === 0 ? undefined : `${counts.failure} failed`,
    counts.pending === 0 ? undefined : `${counts.pending} pending`,
    counts.success === 0 ? undefined : `${counts.success} passed`,
    counts.neutral === 0 ? undefined : `${counts.neutral} skipped or neutral`,
  ].filter((part): part is string => part !== undefined).join(' · ') || 'No checks'
}

export function CheckList({ label, runs, now }: { readonly label: string; readonly runs: readonly CheckRow[]; readonly now: number }) {
  return <ul aria-label={label} {...stylex.props(d.rows)}>{runs.map((run, index) => {
    const seconds = durationSeconds(run.started_at, run.completed_at, now)
    const tone = checkTone(run)
    return <li key={`${run.name}:${index}`} data-check-tone={tone} {...stylex.props(d.row)}>
      <div {...stylex.props(d.rowMain)}>
        <span {...stylex.props(d.word, toneStyle(tone))}>{checkWord(run)}</span>
        <span {...stylex.props(d.strong)}>{run.name}</span>
        {run.workflow == null ? null : <span {...stylex.props(d.muted)}>{run.workflow}</span>}
      </div>
      <div {...stylex.props(d.rowSide)}>
        {seconds === undefined ? null : <span data-duration>{formatDurationShort(seconds)}</span>}
        <ExternalLink href={run.url} label={`Logs for ${run.name} on GitHub`}>Logs on GitHub</ExternalLink>
      </div>
      {run.summary == null || run.summary === '' ? null : <p {...stylex.props(d.rowSummary)}>{run.summary}</p>}
    </li>
  })}</ul>
}
