import { useAtomValue } from '@effect/atom-react'
import * as stylex from '@stylexjs/stylex'
import { DateTime, Option } from 'effect'
import * as React from 'react'

import { scale, tokens } from '../../ui-compat/tokens.stylex.ts'
import { borderVars as border, colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as status, surfaceVars as surface, typeVars as t } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { Icon } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { LiveRing } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/LiveRing.tsx'
import { NarrowLayoutContext } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { HumanTime } from '@fractal/ui/human-time'

import { useDataSource } from '../../data/react.tsx'
import { credentialExpiredMessage } from '../../data/source.ts'
import { agentTodosAtom, todoTextParts, type TodoState } from './model.ts'

type Task = Extract<TodoState, { readonly _tag: 'Observed' }>['todo']['snapshot']['phases'][number]['tasks'][number]

const occurrenceKey = ({ counts, parts }: { counts: Map<string, number>; parts: readonly string[] }): string => {
  const key = JSON.stringify(parts)
  const occurrence = counts.get(key) ?? 0
  counts.set(key, occurrence + 1)
  return `${key}:${occurrence}`
}

const styles = stylex.create({
  resources: { color: 'inherit', fontSize: '0.75rem' },
  pinned: {
    width: '100%',
    minWidth: 0,
    boxSizing: 'border-box',
    paddingInline: s.section,
    flexShrink: 0,
  },
  detail: { padding: scale.space4, color: tokens['--ds-gray-1000'], fontSize: '0.8125rem' },
  strip: {
    maxWidth: g.lane,
    marginInline: 'auto',
    color: tokens['--ds-gray-1000'],
    fontSize: '0.75rem',
    borderTopWidth: 1,
    borderTopStyle: 'solid',
    borderTopColor: tokens['--ds-gray-alpha-400'],
  },
  summary: {
    cursor: 'pointer',
    paddingBlock: s.md,
    minWidth: 0,
    outline: { default: 'none', ':focus-visible': `2px solid ${tokens['--ds-blue-700']}` },
    borderRadius: scale.radiusDefault,
  },
  summaryTouch: { minHeight: 44, boxSizing: 'border-box', paddingBlock: 14 },
  summaryRow: {
    display: 'inline-flex',
    verticalAlign: 'middle',
    width: 'calc(100% - 18px)',
    alignItems: 'center',
    gap: s.md,
    minWidth: 0,
  },
  phase: {
    flexGrow: 1,
    minWidth: 0,
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
  },
  count: { fontVariantNumeric: 'tabular-nums', flexShrink: 0 },
  source: { color: tokens['--ds-gray-900'], flexShrink: 0, fontSize: '0.6875rem' },
  stale: { color: tokens['--ds-amber-900'] },
  body_: { maxHeight: 'min(40vh, 320px)', overflowY: 'auto', paddingBottom: scale.space2 },
  // The plan, focus first: the current phase's work is open, everything finished folds away.
  plan: { display: 'flex', flexDirection: 'column', gap: s.lg, minWidth: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, color: c.fg },
  eyebrow: { display: 'flex', justifyContent: 'space-between', gap: s.md, fontSize: t.denseSize, lineHeight: t.metaLeading, color: c.fgMuted },
  tabular: { fontVariantNumeric: 'tabular-nums', flexShrink: 0 },
  phaseName: { margin: 0, marginBlock: `${s.xs2} ${s.md}`, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium, overflowWrap: 'anywhere' },
  bar: { display: 'flex', gap: s.xs2, height: s.sm },
  segment: { display: 'flex', minWidth: s.xs2, overflow: 'hidden', borderRadius: r.full, backgroundColor: border.rail },
  fill: { display: 'block', height: '100%' },
  fillDone: { backgroundColor: c.fgSoft },
  fillActive: { backgroundColor: status.running },
  // Hatched, not coloured: blocked says nothing about who has to act.
  fillBlocked: { backgroundImage: `repeating-linear-gradient(135deg, ${c.fgMuted} 0 2px, transparent 2px 4px)` },
  now: { padding: s.md, backgroundColor: surface.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md },
  label: { margin: 0, marginBottom: s.xs, fontSize: t.denseSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium, color: c.fgMuted },
  list: { listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: s.xs2 },
  task: { display: 'flex', gap: s.md, paddingBlock: s.xs2, minWidth: 0 },
  slot: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.icon, height: t.metaLeading, flexShrink: 0, color: c.fgMuted },
  body: { flexGrow: 1, minWidth: 0, overflowWrap: 'anywhere' },
  textOutstanding: { color: c.fgSoft },
  textDone: { color: c.fgMuted },
  chip: { display: 'inline-flex', alignItems: 'center', gap: s.xs2, verticalAlign: '-1px', maxWidth: '100%', paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, backgroundColor: surface.controlFill, fontFamily: t.fontMono, fontSize: t.denseSize, whiteSpace: 'nowrap' },
  note: { margin: 0, marginTop: s.xs2, fontSize: t.denseSize, lineHeight: t.metaLeading, color: c.fgSoft, overflowWrap: 'anywhere' },
  noteLead: { fontWeight: t.weightMedium, color: c.fg },
  fold: { minWidth: 0 },
  foldSummary: { display: 'flex', alignItems: 'flex-start', gap: s.sm, minHeight: g.controlSm, boxSizing: 'border-box', paddingBlock: s.xs, marginInline: `calc(-1 * ${s.xs})`, paddingInline: s.xs, borderRadius: r.sm, color: c.fgSoft, cursor: 'pointer', listStyle: 'none', outline: 'none', ':hover': { backgroundColor: c.rowHover, color: c.fg }, ':focus-visible': { outline: `${g.focusRing} solid ${c.primary}` }, '::-webkit-details-marker': { display: 'none' } },
  foldTouch: { minHeight: g.touchTarget, alignItems: 'center' },
  foldChevron: { display: 'inline-flex', alignItems: 'center', height: t.metaLeading, flexShrink: 0, color: c.fgMuted },
  foldLabel: { flexGrow: 1, minWidth: 0, overflowWrap: 'anywhere' },
  foldCount: { flexShrink: 0, color: c.fgMuted, fontSize: t.denseSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums' },
  others: { display: 'flex', flexDirection: 'column', paddingTop: s.sm, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: c.border },
  foot: { display: 'flex', flexWrap: 'wrap', justifyContent: 'space-between', gap: `${s.xs2} ${s.row}`, paddingTop: s.md, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: c.border, fontSize: t.denseSize, lineHeight: t.metaLeading, color: c.fgMuted },
  footNotes: { flexBasis: '100%', display: 'flex', flexDirection: 'column', gap: s.xs2 },
})

const statusLabels = {
  pending: 'Pending',
  in_progress: 'In progress',
  completed: 'Completed',
  blocked: 'Blocked',
}

const StatusMark = ({ status: taskStatus, live }: { readonly status: Task['status']; readonly live: boolean }) =>
  <span role="img" aria-label={typeof taskStatus === 'string' ? statusLabels[taskStatus] : 'Unrecognized status'} {...stylex.props(styles.slot)}>
    {taskStatus === 'in_progress' ? <LiveRing size="sm" spinning={live} />
      : taskStatus === 'completed' ? <Icon name="check" size={12} />
        : taskStatus === 'blocked' ? <svg width={14} height={14} viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeLinecap="round" aria-hidden="true"><circle cx="7" cy="7" r="6" /><path d="M5.5 4.75v4.5M8.5 4.75v4.5" strokeWidth={1.25} /></svg>
          : taskStatus === 'pending' ? <svg width={12} height={12} viewBox="0 0 12 12" fill="none" stroke="currentColor" aria-hidden="true"><circle cx="6" cy="6" r="4.5" /></svg>
            : '?'}
  </span>

const chipIcons = { PullRequest: 'pull-request', Commit: 'dot', Agent: 'person' } as const
const chipWords = { PullRequest: 'Pull request', Commit: 'Commit', Agent: 'Agent' } as const

/** Plain text with its references as chips. They label for now: a bare #123 has no repository to open against. */
const TaskText = ({ text }: { readonly text: string }) => <>{todoTextParts(text).map((part, index) => part._tag === 'Text'
  ? <React.Fragment key={index}>{part.text}</React.Fragment>
  : <span key={index} title={`${chipWords[part._tag]} ${part.text}`} data-todo-ref={part._tag} {...stylex.props(styles.chip)}><Icon name={chipIcons[part._tag]} size={10} />{part._tag === 'Agent' ? part.text.split('/').at(-1) : part.text}</span>)}</>

const TaskRows = ({ tasks, live }: { readonly tasks: readonly Task[]; readonly live: boolean }) => {
  const occurrences = new Map<string, number>()
  return <ul {...stylex.props(styles.list)}>
    {tasks.map(task => <li key={occurrenceKey({ counts: occurrences, parts: [task.content] })} data-todo-status={typeof task.status === 'string' ? task.status : 'unknown'} {...stylex.props(styles.task)}>
      <StatusMark status={task.status} live={live} />
      <span {...stylex.props(styles.body)}>
        <span {...stylex.props(task.status === 'completed' ? styles.textDone : task.status === 'pending' && styles.textOutstanding)}><TaskText text={task.content} /></span>
        {Option.isSome(task.blocker) ? <p {...stylex.props(styles.note)}>
          {task.status === 'completed' ? <>Earlier blocker: <TaskText text={task.blocker.value} /></> : <><span {...stylex.props(styles.noteLead)}>Blocked</span> · <TaskText text={task.blocker.value} /></>}
        </p> : task.status === 'blocked' ? <p {...stylex.props(styles.note)}><span {...stylex.props(styles.noteLead)}>Blocked</span></p> : null}
      </span>
    </li>)}
  </ul>
}

/** A native disclosure: it stays in the page for find-in-page and needs no state to keep the plan short. */
const Fold = ({ label, count, children }: { readonly label: React.ReactNode; readonly count?: string; readonly children: React.ReactNode }) => {
  const narrow = React.useContext(NarrowLayoutContext)
  const [open, setOpen] = React.useState(false)
  return <details onToggle={event => setOpen(event.currentTarget.open)} {...stylex.props(styles.fold)}>
    <summary {...stylex.props(styles.foldSummary, narrow && styles.foldTouch)}>
      <span {...stylex.props(styles.foldChevron)}><Icon name={open ? 'chevron-down' : 'chevron-right'} size={12} /></span>
      <span {...stylex.props(styles.foldLabel)}>{label}</span>
      {count === undefined ? null : <span {...stylex.props(styles.foldCount)}>{count}</span>}
    </summary>
    {children}
  </details>
}

const tally = (tasks: readonly Task[]) => ({
  done: tasks.filter(task => task.status === 'completed').length,
  active: tasks.filter(task => task.status === 'in_progress').length,
  blocked: tasks.filter(task => task.status === 'blocked').length,
  // Abandoned work is reported only in the totals, but a task may still arrive with an unknown status.
  counted: tasks.length,
})

const ProgressSegment = ({ done, active, blocked, counted }: ReturnType<typeof tally>) =>
  <span style={{ flex: `${Math.max(counted, 1)} 1 0` }} {...stylex.props(styles.segment)}>
    {([[done, styles.fillDone], [active, styles.fillActive], [blocked, styles.fillBlocked]] as const).map(([value, tone], index) =>
      value === 0 ? null : <i key={index} style={{ width: `${100 * value / counted}%` }} {...stylex.props(styles.fill, tone)} />)}
  </span>

/** T1, focus-first: where it is now, what is next in this phase, and everything else behind a fold. */
const FocusPlan = ({ state }: { readonly state: Extract<TodoState, { readonly _tag: 'Observed' }> }) => {
  const narrow = React.useContext(NarrowLayoutContext)
  const { todo, completed, total, phase, binding } = state
  const { snapshot } = todo
  const live = binding === 'current'
  const { totals } = snapshot
  const current = phase === undefined ? -1 : snapshot.phases.indexOf(phase)
  const by = (wanted: Task['status']) => phase?.tasks.filter(task => task.status === wanted) ?? []
  const now = [...by('in_progress'), ...by('blocked')]
  const next = phase?.tasks.filter(task => task.status !== 'completed' && task.status !== 'in_progress' && task.status !== 'blocked') ?? []
  const doneHere = by('completed')
  const others = snapshot.phases.map((listed, index) => ({ listed, index })).filter(({ index }) => index !== current)
  const abandoned = totals.abandoned ?? 0
  // A truncated list cannot draw phases to scale, so the bar falls back to the full-source totals.
  const segments = snapshot.truncated
    ? [{ done: totals.completed, active: totals.in_progress, blocked: totals.blocked, counted: Math.max(total, 1) }]
    : snapshot.phases.map(listed => tally(listed.tasks))
  const phaseOccurrences = new Map<string, number>()
  return <div data-todo-plan {...stylex.props(styles.plan)}>
    {snapshot.phases.length === 0 ? <p {...stylex.props(styles.note)}>No to-dos in this list.</p> : <div>
      <div {...stylex.props(styles.eyebrow)}>
        <span>{current === -1 ? 'All phases finished' : `Phase ${current + 1} of ${snapshot.phases.length}`}</span>
        <span {...stylex.props(styles.tabular)}>{completed} of {total} done</span>
      </div>
      {phase === undefined ? null : <p {...stylex.props(styles.phaseName)}>{phase.name}</p>}
      <div role="img" aria-label={`${completed} of ${total} done, ${totals.in_progress} in progress, ${totals.blocked} blocked, ${totals.pending} pending`} {...stylex.props(styles.bar)}>
        {segments.map((segment, index) => <ProgressSegment key={index} {...segment} />)}
      </div>
    </div>}
    {now.length === 0 ? null : <section aria-label="Now" {...stylex.props(styles.now)}><p {...stylex.props(styles.label)}>Now</p><TaskRows tasks={now} live={live} /></section>}
    {next.length === 0 ? null : <section aria-label="Next in this phase"><p {...stylex.props(styles.label)}>Next in this phase</p><TaskRows tasks={next} live={live} /></section>}
    {doneHere.length === 0 ? null : <Fold label={`${doneHere.length} done in this phase`}><TaskRows tasks={doneHere} live={live} /></Fold>}
    {others.length === 0 ? null : <div {...stylex.props(styles.others)}>
      {others.map(({ listed }) => {
        const { done, blocked } = tally(listed.tasks)
        return <Fold key={occurrenceKey({ counts: phaseOccurrences, parts: [listed.name] })} label={listed.name} count={`${done}/${listed.tasks.length}${blocked === 0 ? '' : ` · ${blocked} blocked`}`}>
          <TaskRows tasks={listed.tasks} live={live} />
        </Fold>
      })}
    </div>}
    <div {...stylex.props(styles.foot)}>
      {snapshot.truncated || binding !== 'current' ? <div {...stylex.props(styles.footNotes)}>
        {snapshot.truncated ? <span>Only some to-dos are shown. Progress counts the full list.</span> : null}
        {binding !== 'current' ? <span>{binding === 'pending' ? 'Checking to-dos…' : 'These to-dos may be out of date.'}</span> : null}
      </div> : null}
      {abandoned > 0 ? <span>{abandoned} abandoned, not counted</span> : null}
      <HumanTime at={DateTime.toEpochMillis(snapshot.observed_at)} prefix="Updated " tooltip={!narrow} />
    </div>
  </div>
}

/** Read-only accepted harness facts; no task mutation or transcript-derived authority. */
export const AgentTodos = ({
  state,
  detail = false,
  placement = 'pinned',
}: {
  readonly state: TodoState
  readonly detail?: boolean
  /** `resources`: a section of the Resources view; it names every state instead of hiding unobserved ones. */
  readonly placement?: 'pinned' | 'resources'
}): React.ReactNode => {
  const narrow = React.useContext(NarrowLayoutContext)
  if (state._tag !== 'Observed') {
    if (!detail && placement !== 'resources' && state._tag !== 'Unauthorized') return null
    return (
      <p role="status" {...stylex.props(placement === 'resources' ? styles.resources : styles.detail)}>
        {state._tag === 'Waiting'
          ? 'Loading to-dos…'
          : state._tag === 'Unauthorized' ? credentialExpiredMessage : state._tag === 'Unavailable'
            ? "To-dos aren't available right now."
            : 'This session has not shared a to-do list.'}
      </p>
    )
  }
  const { todo, completed, total, phase, binding } = state
  const stale = binding === 'stale'
  const snapshot = todo.snapshot
  const label =
    phase?.name ??
    (total === 0
      ? 'No to-dos'
      : completed === total
        ? 'Complete'
        : 'To-dos in progress')
  if (detail) return <section aria-label="To-dos" {...stylex.props(styles.detail)}><FocusPlan state={state} /></section>
  if (placement === 'resources') return <div {...stylex.props(styles.resources)}><FocusPlan state={state} /></div>
  return (
    <div {...stylex.props(styles.pinned)}>
      <details aria-label="To-dos" {...stylex.props(styles.strip)}>
        <summary {...stylex.props(styles.summary, narrow && styles.summaryTouch)}>
          <span {...stylex.props(styles.summaryRow)}>
            <span title={label} {...stylex.props(styles.phase)}>
              To-dos · {label}
            </span>
            <span
              aria-label={`${completed} of ${total} to-dos completed`}
              {...stylex.props(styles.count)}
            >
              {completed}/{total}
            </span>
            {binding !== 'current' || snapshot.truncated ? <span {...stylex.props(styles.source, stale && styles.stale)}>
              {binding === 'pending' ? 'Checking' : stale ? 'May be out of date' : ''}
              {snapshot.truncated ? `${binding !== 'current' ? ' · ' : ''}Some shown` : ''}
            </span> : null}
          </span>
        </summary>
        <div {...stylex.props(styles.body_)}><FocusPlan state={state} /></div>
      </details>
    </div>
  )
}

/** Renders the shared todo projection for an agent from the active gateway data source. */
export const LiveAgentTodos = ({
  agentRef,
  detail = false,
  placement,
}: {
  readonly agentRef: string
  readonly detail?: boolean
  readonly placement?: 'pinned' | 'resources'
}): React.ReactNode => {
  const source = useDataSource()
  const state = useAtomValue(agentTodosAtom({ source, agentRef }))
  return <AgentTodos state={state} detail={detail} {...(placement === undefined ? {} : { placement })} />
}
