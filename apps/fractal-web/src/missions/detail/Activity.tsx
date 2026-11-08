import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t, interactionVars } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { missionHistory, span, type HistoryEntry, type MissionDetailModel, type Problem } from './detailModel.ts'
import type { MissionFocusControls } from './focus.ts'
import { clock } from './traceScale.ts'

// Below the graph and the trace: everything that went wrong or stalled, and what happened, both newest first. Each
// row picks its step in every view; hovering one shows it in the inspector.

const PROBLEM_WORD: Record<Problem['kind'], string> = { failure: 'Failed', retry: 'Retried', 'wait-you': 'Waited on you', stall: 'Stalled' }

export const ProblemsList = ({ model, controls }: { readonly model: MissionDetailModel; readonly controls: MissionFocusControls }) => {
  const problems = [...model.problems].reverse()
  if (problems.length === 0) return <p data-problems-empty="" {...stylex.props(styles.empty)}>Nothing has gone wrong or stalled on this run.</p>
  return <ol aria-label="Problems" {...stylex.props(styles.list)}>
    {problems.map((problem, index) => <li key={`${problem.step}:${problem.kind}:${index}`} data-problem={problem.kind}>
      <Aria.Button onPress={() => controls.select(problem.step)} onHoverStart={() => controls.hover(problem.step)} onHoverEnd={() => controls.hover(undefined)} {...stylex.props(styles.row)}>
        <span {...stylex.props(styles.kind, problem.kind === 'failure' && styles.bad, problem.kind === 'wait-you' && problem.resolvedAt === undefined && styles.you)}>{PROBLEM_WORD[problem.kind]}</span>
        <span {...stylex.props(styles.text)}>{problem.text}</span>
        <span {...stylex.props(styles.when)}>{problem.resolvedAt === undefined ? `${clock(problem.at)} · open` : `${clock(problem.at)} · resolved after ${span(problem.resolvedAt - problem.at)}`}</span>
      </Aria.Button>
    </li>)}
  </ol>
}

const HISTORY_WORD: Record<HistoryEntry['kind'], string> = { started: 'Started', done: 'Done', failed: 'Failed', asked: 'Asked', answered: 'Answered', run: 'Run' }

export const HistoryList = ({ model, controls }: { readonly model: MissionDetailModel; readonly controls: MissionFocusControls }) => {
  const entries = missionHistory(model)
  if (entries.length === 0) return <p {...stylex.props(styles.empty)}>Nothing with a time has happened on this run yet.</p>
  return <>
    <ol aria-label="History" {...stylex.props(styles.list)}>
      {entries.map((entry, index) => <li key={`${entry.at}:${entry.kind}:${index}`} data-history={entry.kind}>
        {entry.step === undefined
          ? <span {...stylex.props(styles.row, styles.static)}><span {...stylex.props(styles.kind)}>{HISTORY_WORD[entry.kind]}</span><span {...stylex.props(styles.text)}>{entry.text}</span><span {...stylex.props(styles.when)}>{clock(entry.at)}</span></span>
          : <Aria.Button onPress={() => controls.select(entry.step!)} onHoverStart={() => controls.hover(entry.step)} onHoverEnd={() => controls.hover(undefined)} {...stylex.props(styles.row)}>
            <span {...stylex.props(styles.kind, entry.kind === 'failed' && styles.bad)}>{HISTORY_WORD[entry.kind]}</span><span {...stylex.props(styles.text)}>{entry.text}</span><span {...stylex.props(styles.when)}>{clock(entry.at)}</span>
          </Aria.Button>}
      </li>)}
    </ol>
    <p {...stylex.props(styles.empty)}>Plan revisions (steps added, removed or rewritten) are not served yet.</p>
  </>
}

const styles = stylex.create({
  list: { display: 'flex', flexDirection: 'column', margin: 0, padding: 0, listStyle: 'none' },
  row: { display: 'flex', flexWrap: 'wrap', alignItems: 'baseline', columnGap: s.md, rowGap: 0, width: '100%', minHeight: g.controlMd, paddingInline: s.sm, paddingBlock: s.xs, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fg, fontFamily: 'inherit', fontSize: t.metaSize, textAlign: 'start', cursor: 'pointer',
    ':hover': { backgroundColor: surface.rowHover }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing } },
  static: { cursor: 'default', ':hover': { backgroundColor: 'transparent' } },
  kind: { flexShrink: 0, minWidth: '6.5rem', color: ink.fgMuted, fontSize: t.denseSize },
  text: { flex: '1 1 14rem', minWidth: 0, overflowWrap: 'anywhere' },
  when: { flexShrink: 0, color: ink.fgMuted, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  you: { color: `color-mix(in srgb, ${tone.attention} 80%, ${ink.fg})` },
  bad: { color: `color-mix(in srgb, ${tone.dangerFg} 80%, ${ink.fg})` },
  empty: { margin: 0, paddingBlock: s.sm, color: ink.fgMuted, fontSize: t.metaSize },
})
