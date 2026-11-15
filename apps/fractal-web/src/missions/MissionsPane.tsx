import { MissionEtaLabel, formatMissionEta } from '@fractal/ui/assistant-ui/shell'
import type * as Fractal from '@fractal/client/schema'
import * as stylex from '@stylexjs/stylex'
import { DateTime } from 'effect'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { ago } from '../../../../clients/typescript/fractal-views/time.ts'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useMissionsDecode, useNow } from '../data/react.tsx'
import { LiveMissionThreads } from '../threads/LiveThreads.tsx'
import { DecodeNotice } from '../web/LiveRoster.tsx'
import { MissionFeedNotice, missionsTruncatedText } from './MissionFeedNotice.tsx'
import { hiddenByDefault, missionEta, missionReaderTitle, type MissionPresentation, type PresentedStep, type StepWord } from './presentation.ts'
import { missionWord } from './missionWords.ts'
import { startOfDay, useMissionRows } from './useMissionRows.ts'
import { StepSourceChip } from './StepSourceChip.tsx'
import { DeclarationDialog, type DeclarationTarget } from '../web/DeclarationDialog.tsx'

// The Missions screen: every mission with its word and who must act, in the contract's order,
// as the Fractal terminal UI's Missions tab lists them. A row opens to its steps. Fractal's own missions, ones never
// started and ones settled before today are folded away until "Show all".

const STEP_GLYPH: Record<StepWord, string> = { needs_you: '◆', done: '✓', working: '⠋', ready: '◇', waiting: '◐', cancelled: '⊘', failed: '✕', pending: '○' }
const STEP_NAME: Record<StepWord, string> = { needs_you: 'needs you', done: 'done', working: 'working', ready: 'ready', waiting: 'waiting', cancelled: 'cancelled', failed: 'failed', pending: 'pending' }

export const MissionsPane = ({ onOpenAgent, onOpenThread, focusedMission, onFocusMission }: {
  readonly onOpenAgent?: (ref: string) => void
  /** Opens one of a mission's threads in its own tab. */
  readonly onOpenThread?: ((ref: string) => void) | undefined
  /** The URL's mission: opened and scrolled to. Without a host this screen keeps its own open row. */
  readonly focusedMission?: string
  readonly onFocusMission?: (ref: string | undefined) => void
}): React.ReactNode => {
  const { missions, rows, observedIds } = useMissionRows()
  const missionsDecode = useMissionsDecode()
  const now = useNow()
  const [all, setAll] = React.useState(false)
  const [localOpen, setLocalOpen] = React.useState<string>()
  // A step chip opens the declaration sheet at its run's pinned revision.
  const [source, setSource] = React.useState<DeclarationTarget>()
  const open = onFocusMission === undefined ? localOpen : focusedMission
  const setOpen = onFocusMission ?? setLocalOpen
  const today = startOfDay(now)
  // A focused mission is shown even when it would be folded away.
  const shown = all ? rows : rows.filter((row) => row.mission.id === open || !hiddenByDefault(row.presentation, today))
  const hidden = rows.length - shown.length
  const list = React.useRef<HTMLUListElement>(null)
  React.useEffect(() => {
    if (open !== undefined) [...list.current?.querySelectorAll<HTMLElement>('[data-mission-id]') ?? []].find((row) => row.dataset.missionId === open)?.scrollIntoView?.({ block: 'nearest' })
  }, [open, rows.length])
  return (
    <section aria-label="Missions" data-testid="missions-pane" {...stylex.props(styles.pane)}>
      <header {...stylex.props(styles.header)}>
        <strong>Missions</strong>
        {missions._tag === 'Observed' && missions.freshness === 'stale' ? <span {...stylex.props(styles.meta)}>last observed · live updates pending</span> : null}
        {hidden > 0 || all ? (
          <Aria.Button onPress={() => setAll(!all)} {...stylex.props(styles.button)}>
            {all ? 'Hide settled and Fractal’s own' : `Show all (${hidden} more)`}
          </Aria.Button>
        ) : null}
        {missions._tag === 'Observed' ? <DecodeNotice noun="mission" decode={missionsDecode} shown={observedIds} xstyle={styles.decode} /> : null}
        {missions._tag === 'Observed' && missionsDecode.hasMore === true ? <p role="status" {...stylex.props(styles.meta, styles.truncated)}>{missionsTruncatedText(missions.value.length)}</p> : null}
      </header>
      <div {...stylex.props(styles.body)}>
        {shown.length === 0 || missions._tag !== 'Observed' ? <MissionFeedNotice missions={missions} total={rows.length} shown={shown.length} />
          : (
                <ul ref={list} {...stylex.props(styles.list)}>
                  {shown.map(({ mission, presentation }) => {
                    const word = missionWord(presentation.word)
                    const expanded = open === mission.id
                    // Served as of its publication cut, never counted down; a settled mission shows none.
                    const eta = presentation.settled_at === null ? missionEta(mission) : null
                    return (
                      <li key={mission.id} data-mission-id={mission.id} {...stylex.props(styles.item)}>
                        <Aria.Button aria-expanded={expanded} onPress={() => setOpen(expanded ? undefined : mission.id)} {...stylex.props(styles.row)}>
                          <span aria-hidden="true" {...stylex.props(styles.glyph, presentation.word === 'decision' && styles.person, ['stalled', 'failed'].includes(presentation.word) && styles.fault)}>{word.glyph}</span>
                          <span {...stylex.props(styles.word)} title={word.explain}>{word.name}</span>
                          <span {...stylex.props(styles.title)}>{missionReaderTitle(mission)}</span>
                          <span {...stylex.props(styles.meta)}>
                            {presentation.who.kind === 'nobody' ? '' : `${presentation.who.label} · `}
                            {presentation.progress.total > 0 ? `${presentation.progress.done}/${presentation.progress.total} · ` : ''}
                            {formatMissionEta(eta) === null ? null : <><MissionEtaLabel eta={eta} now={now} />{' · '}</>}
                            {ago(DateTime.formatIso(mission.updated_at), now)}
                          </span>
                        </Aria.Button>
                        {presentation.last_progress && !expanded ? <div {...stylex.props(styles.progress)}>{presentation.last_progress}</div> : null}
                        {expanded ? <MissionDetail mission={mission} presentation={presentation} onOpenAgent={onOpenAgent} onOpenThread={onOpenThread} onOpenSource={setSource} /> : null}
                      </li>
                    )
                  })}
                </ul>
              )}
      </div>
      {/* Always mounted: the theme portal host must exist before the sheet opens; contents render only while open. */}
      <DeclarationDialog subject={source?.subject ?? ''} revision={source?.revision} node={source?.node} context={source?.context} isOpen={source !== undefined} onOpenChange={(isOpen) => { if (!isOpen) setSource(undefined) }} />
    </section>
  )
}

const MissionDetail = ({ mission, presentation, onOpenAgent, onOpenThread, onOpenSource }: { readonly mission: Fractal.Mission; readonly presentation: MissionPresentation; readonly onOpenAgent?: (ref: string) => void; readonly onOpenThread?: ((ref: string) => void) | undefined; readonly onOpenSource: (target: DeclarationTarget) => void }): React.ReactNode => {
  const word = missionWord(presentation.word)
  return (
    <div {...stylex.props(styles.detail)} aria-label={`${missionReaderTitle(mission)} steps`}>
      <div {...stylex.props(styles.meta)}>{word.explain} · {mission.id}</div>
      {presentation.stuck?.agent ? (
        <div {...stylex.props(styles.stuck)}>
          Stuck at {presentation.stuck.step.split('/').pop()}{presentation.stuck.attempts > 1 ? ` after ${presentation.stuck.attempts} attempts` : ''}; its seat {presentation.stuck.agent.label} is {presentation.stuck.agent.state}.
        </div>
      ) : null}
      {presentation.last_progress ? <div {...stylex.props(styles.progress)}>Last progress: {presentation.last_progress}</div> : null}
      {presentation.steps.length === 0 ? <p {...stylex.props(styles.notice)}>st did not send this mission's steps.</p> : (
        <ol {...stylex.props(styles.steps)}>
          {presentation.steps.map((step) => <StepRow key={step.id} mission={mission.id} step={step} onOpenAgent={onOpenAgent} onOpenSource={onOpenSource} />)}
        </ol>
      )}
      <LiveMissionThreads missionId={mission.id} onOpenThread={onOpenThread} />
    </div>
  )
}

const StepRow = ({ mission, step, onOpenAgent, onOpenSource }: { readonly mission: string; readonly step: PresentedStep; readonly onOpenAgent?: (ref: string) => void; readonly onOpenSource: (target: DeclarationTarget) => void }): React.ReactNode => (
  <li {...stylex.props(styles.step)}>
    <span aria-hidden="true" {...stylex.props(styles.glyph, step.word === 'needs_you' && styles.person, step.word === 'failed' && styles.fault)}>{STEP_GLYPH[step.word]}</span>
    <span {...stylex.props(styles.stepWord)}>{STEP_NAME[step.word]}</span>
    <span {...stylex.props(styles.title)}>{step.title}</span>
    {step.owner.kind === 'agent' && step.owner.agent !== undefined && onOpenAgent !== undefined
      ? <Aria.Button onPress={() => onOpenAgent(step.owner.agent!)} {...stylex.props(styles.link)}>{step.owner.label}</Aria.Button>
      : <span {...stylex.props(styles.meta)}>{step.owner.label}</span>}
    {step.attempts > 1 ? <span {...stylex.props(styles.meta)}>attempt {step.attempts}</span> : null}
    {step.revision === null ? null : <StepSourceChip mission={mission} revision={step.revision} step={step.path} onOpen={onOpenSource} />}
    {step.note ? <span {...stylex.props(styles.note)}>{step.note}</span> : null}
  </li>
)

const styles = stylex.create({
  pane: { position: 'absolute', inset: 0, zIndex: 2, display: 'flex', flexDirection: 'column', minHeight: 0, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize },
  header: { display: 'flex', flexWrap: 'wrap', alignItems: 'baseline', gap: s.md, paddingInline: s.lg, paddingBlock: s.md, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: c.border, flexShrink: 0 },
  decode: { flexBasis: '100%', padding: 0 },
  truncated: { flexBasis: '100%', margin: 0, whiteSpace: 'normal' },
  body: { flexGrow: 1, minHeight: 0, overflowY: 'auto', padding: s.md },
  button: { marginInlineStart: 'auto', padding: '4px 8px', borderWidth: 0, borderRadius: 6, backgroundColor: 'transparent', color: c.fgMuted, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover } },
  list: { margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column', gap: 2 },
  item: { borderRadius: 6 },
  row: { width: '100%', display: 'grid', gridTemplateColumns: '1.25rem 6.5rem minmax(0, 1fr) auto', alignItems: 'baseline', columnGap: s.md, padding: s.md, borderWidth: 0, borderRadius: 6, backgroundColor: 'transparent', color: c.fg, textAlign: 'start', fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `2px solid ${c.primary}` } },
  glyph: { color: c.fgMuted, textAlign: 'center' },
  person: { color: c.attention },
  fault: { color: c.dangerFg },
  word: { color: c.fgMuted },
  title: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  meta: { color: c.fgMuted, fontSize: t.denseSize, whiteSpace: 'nowrap' },
  progress: { paddingInlineStart: 'calc(1.25rem + 6.5rem + 24px)', color: c.fgMuted, fontSize: t.denseSize, overflowWrap: 'anywhere' },
  detail: { display: 'flex', flexDirection: 'column', gap: s.sm, paddingBlock: s.sm, paddingInlineStart: 'calc(1.25rem + 16px)', paddingInlineEnd: s.md },
  stuck: { color: c.dangerFg, fontSize: t.metaSize },
  steps: { margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column', gap: 2 },
  step: { display: 'flex', flexWrap: 'wrap', alignItems: 'baseline', columnGap: s.md, fontSize: t.metaSize },
  stepWord: { color: c.fgMuted, minWidth: '5.5rem' },
  link: { padding: 0, borderWidth: 0, backgroundColor: 'transparent', color: c.primary, fontSize: t.denseSize, cursor: 'pointer', textDecoration: 'underline' },
  note: { flexBasis: '100%', paddingInlineStart: 'calc(1.25rem + 5.5rem + 24px)', color: c.fgMuted, fontSize: t.denseSize },
  notice: { color: c.fgMuted, fontSize: t.metaSize, padding: s.md },
})
