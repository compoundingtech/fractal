import { inlineFromPlain, markdownSource, type MarkdownInline } from '@fractal/client/markdown'
import { useAtomValue } from '@effect/atom-react'
import type * as Fractal from '@fractal/client/schema'
import type { DecodedAgent } from '@fractal/sdk/read'
import { MissionEtaLabel, MissionProgressMark, MissionStateMark, compactAge, type MissionRowTone } from '@fractal/ui/assistant-ui/shell'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { InlineMarkdown } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Markdown.tsx'
import { LiveRing } from '../../../../packages/fractal-ui/src/assistant-ui/composition/LiveRing.tsx'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { formatDuration } from '../../../../packages/fractal-ui/src/time/human-time.ts'
import { leafText } from '../data/projections.ts'
import { useDataSource, useGrants, useNow } from '../data/react.tsx'
import type { Feed } from '../data/source.ts'
import { MissionActionControl, useMissionActions, type MissionActions } from './MissionActionControl.tsx'
import { missionActionOffers, type MissionActionOffer, type MissionActionPort } from './missionActions.ts'
import { MissionFeedNotice } from './MissionFeedNotice.tsx'
import { missionActivity, missionPlan, stepDuration, type ActivityEntry, type MissionPlan, type PlanStep } from './missionPlan.ts'
import { missionEta, missionReaderTitle, presentMission, type StepWord } from './presentation.ts'
import { missionAgents, missionRowData } from './rowData.ts'

// A mission as its own tab (variant A of the detail exploration): a compact header with state, progress, ETA and
// agents; an amber banner naming why it needs the person (red only for failure) with the one action; the plan as a
// mini dependency graph over a vertical stepper; then agents, activity, linked work, the KDL and the offered actions.

export interface MissionDetailViewProps {
  /** `mission/<id>`, the tab's ref. */
  readonly missionRef: string
  /** False while the tab is hidden: animations pause. */
  readonly visible: boolean
  /** Opens agents, alerts, linked work and the declaration (`declaration/<missionRef>`). */
  readonly onOpen: (ref: string, gesture?: { readonly background?: boolean; readonly split?: 'right' | 'below' }) => void
}

type AgentInfo = { readonly id: string; readonly label: string; readonly state: string | undefined }

/** Over the live feeds. */
export const MissionDetailView = ({ missionRef, visible, onOpen }: MissionDetailViewProps): React.ReactNode => {
  const source = useDataSource()
  const missions = useAtomValue(source.missions)
  const agentsFeed = useAtomValue(source.agents)
  const attentionFeed = useAtomValue(source.attention)
  const grants = useGrants()
  const now = useNow()
  const agents = agentsFeed._tag === 'Observed' ? agentsFeed.value : []
  const attention = attentionFeed._tag === 'Observed' ? attentionFeed.value.filter(item => item.state === 'open') : []
  return <MissionDetail missionRef={missionRef} visible={visible} onOpen={onOpen} missions={missions} agents={agents} attention={attention} readOnly={grants.actions !== 'granted'} now={now} {...(source.missionActions === undefined ? {} : { actionPort: source.missionActions })} />
}

/** Data in, for stories and tests. */
export const MissionDetail = ({ missionRef, visible, onOpen, missions, agents, attention, readOnly, now, actionPort }: MissionDetailViewProps & {
  readonly missions: Feed<readonly Fractal.Mission[]>
  readonly agents: readonly DecodedAgent[]
  readonly attention: readonly Fractal.Attention[]
  readonly readOnly: boolean
  readonly now: number
  /** Runs Fractal's offered actions; absent, they show why they cannot run here. */
  readonly actionPort?: MissionActionPort
}): React.ReactNode => {
  const mission = missions._tag === 'Observed' ? missions.value.find(item => item.id === missionRef) : undefined
  const presentation = React.useMemo(() => mission === undefined ? undefined : presentMission({ mission, agents, attention }), [mission, agents, attention])
  const plan = React.useMemo(() => mission === undefined || presentation === undefined ? undefined : missionPlan(mission, presentation), [mission, presentation])
  const [expanded, setExpanded] = React.useState<ReadonlySet<string>>(new Set())
  const stepper = React.useRef<HTMLOListElement>(null)
  const actions = useMissionActions({ port: actionPort, mission: mission ?? { id: missionRef, run_generations: {} }, readOnly, onOpen: ref => onOpen(ref) })
  if (missions._tag !== 'Observed') return <div {...stylex.props(styles.notice)}><MissionFeedNotice missions={missions} total={0} shown={0} /></div>
  if (mission === undefined || presentation === undefined || plan === undefined) return <div role="status" data-mission-missing {...stylex.props(styles.notice, styles.missing)}>This mission is not in the current missions feed. It may have been retired, or lie beyond what this view can load.</div>
  const row = { mission, presentation }
  const data = missionRowData(row)
  const stale = missions.freshness === 'stale'
  const mine = attention.filter(item => item.mission_id === mission.id)
  const agentInfo: readonly AgentInfo[] = missionAgents(presentation).map(agent => {
    const live = agents.find(item => item.id === agent.id)
    return { id: agent.id, label: live?.name ?? agent.label, state: live === undefined ? undefined : leafText(live.state) }
  })
  const jumpTo = (stepId: string) => {
    setExpanded(current => new Set([...current, stepId]))
    requestAnimationFrame(() => stepper.current?.querySelector<HTMLElement>(`[data-step-id="${CSS.escape(stepId)}"]`)?.scrollIntoView({ block: 'center', behavior: 'smooth' }))
  }
  const toggle = (stepId: string) => setExpanded(current => { const next = new Set(current); if (!next.delete(stepId)) next.add(stepId); return next })
  const offers = missionActionOffers(mission)
  return <article aria-label={data.title} data-mission-detail={mission.id} data-stale={stale || undefined} {...stylex.props(styles.root)}>
    <header {...stylex.props(styles.header)}>
      <h1 {...stylex.props(styles.title)}>{data.title}</h1>
      <div {...stylex.props(styles.meta)}>
        <span {...stylex.props(styles.metaItem)}><MissionStateMark tone={data.tone} label={data.stateLabel} frozen={stale || !visible} /><span aria-hidden="true">{data.stateLabel}</span></span>
        {data.progress._tag === 'Known' ? <span {...stylex.props(styles.metaItem)}><MissionProgressMark progress={data.progress} retained={stale} /><span aria-hidden="true">steps</span></span> : null}
        {presentation.settled_at === null && missionEta(mission) !== null ? <span {...stylex.props(styles.metaItem)}><MissionEtaLabel eta={missionEta(mission)} now={now} /></span> : null}
        {agentInfo.length > 0 ? <span {...stylex.props(styles.metaItem)}><AgentStack agents={agentInfo} /><span>{agentInfo.slice(0, 2).map(agent => agent.label).join(', ')}{agentInfo.length > 2 ? ` +${agentInfo.length - 2}` : ''}</span></span> : null}
        <span {...stylex.props(styles.metaItem, styles.soft)}>{stale ? 'last observed ' : 'updated '}{compactAge(data.updatedAt, now)} ago</span>
      </div>
      <Banner mission={mission} presentation={presentation} plan={plan} attention={mine} onOpen={onOpen} onJump={jumpTo} offers={offers} actions={actions} />
    </header>
    <div {...stylex.props(styles.body)}>
      <section aria-labelledby="mission-plan" {...stylex.props(styles.section)}>
        <h2 id="mission-plan" {...stylex.props(styles.h2)}>Plan</h2>
        {plan.steps.length === 0 ? <p {...stylex.props(styles.soft)}>Fractal did not send this mission's steps.</p> : <>
          {plan.levels.length > 1 ? <MiniGraph plan={plan} onJump={jumpTo} /> : null}
          <ol ref={stepper} aria-label="Steps" {...stylex.props(styles.stepper)}>
            {plan.levels.map((level, depth) => <li key={depth} data-plan-level={depth} {...stylex.props(styles.level)} style={{ paddingInlineStart: `calc(${Math.min(depth, 6)} * 16px)` }}>
              <ol aria-label={level.length > 1 ? `${level.length} steps in parallel` : undefined} {...stylex.props(styles.levelSteps, level.length > 1 && styles.parallel)}>
                {level.map(step => <StepRow key={step.id} step={step} now={now} expanded={expanded.has(step.id)} onToggle={() => toggle(step.id)} onOpen={onOpen} visible={visible && !stale} compact={level.length > 1} offers={offers.filter(offer => offer.target === step.id)} actions={actions} />)}
              </ol>
            </li>)}
          </ol>
        </>}
      </section>
      {agentInfo.length > 0 ? <section aria-labelledby="mission-agents" {...stylex.props(styles.section)}>
        <h2 id="mission-agents" {...stylex.props(styles.h2)}>Agents</h2>
        <ul {...stylex.props(styles.plainList)}>
          {agentInfo.map(agent => {
            const holding = plan.steps.filter(step => step.owner.kind === 'agent' && step.owner.agent === agent.id && step.word !== 'done')
            return <li key={agent.id}><Aria.Button data-open-ref={agent.id} onPress={event => onOpen(agent.id, event.metaKey || event.ctrlKey ? { background: true } : undefined)} {...stylex.props(styles.linkRow)}>
              <span aria-hidden="true" {...stylex.props(styles.avatar)}>{initials(agent.label)}</span>
              <span {...stylex.props(styles.grow)}>{agent.label}</span>
              <span {...stylex.props(styles.soft)}>{holding.length > 0 ? holding.map(step => step.title).join(', ') : agent.state ?? ''}</span>
              {agent.state !== undefined ? <span {...stylex.props(styles.soft, styles.state)}>{agent.state}</span> : null}
            </Aria.Button></li>
          })}
        </ul>
      </section> : null}
      <Activity entries={missionActivity({ mission, plan, attention: mine })} now={now} onJump={jumpTo} />
      <LinkedWork mission={mission} attention={mine} onOpen={onOpen} />
      <section aria-labelledby="mission-actions" {...stylex.props(styles.section)}>
        <h2 id="mission-actions" {...stylex.props(styles.h2)}>Actions</h2>
        <div {...stylex.props(styles.actions)}>
          {offers.map(offer => <MissionActionControl key={`${offer.id}:${offer.action}:${offer.target}`} offer={offer} actions={actions} />)}
          <Aria.Button data-open-ref={`declaration/${mission.id}`} onPress={event => onOpen(`declaration/${mission.id}`, event.metaKey || event.ctrlKey ? { background: true } : undefined)} {...stylex.props(styles.action)}>View mission KDL</Aria.Button>
        </div>
        {offers.length === 0 ? <p {...stylex.props(styles.soft)}>Fractal offers no actions on this mission right now.</p> : null}
      </section>
    </div>
  </article>
}

const initials = (label: string) => label.split(/[\s-]+/).filter(Boolean).slice(0, 2).map(part => part[0]!.toUpperCase()).join('')

const AgentStack = ({ agents }: { readonly agents: readonly AgentInfo[] }) =>
  <span aria-hidden="true" {...stylex.props(styles.stack)}>{agents.slice(0, 3).map(agent => <span key={agent.id} {...stylex.props(styles.avatar, styles.stacked)}>{initials(agent.label)}</span>)}</span>

/** Amber when the person must act or the mission is stuck, red only for failure; nothing when it is moving. */
const Banner = ({ mission, presentation, plan, attention, onOpen, onJump, offers, actions }: {
  readonly mission: Fractal.Mission
  readonly presentation: ReturnType<typeof presentMission>
  readonly plan: MissionPlan
  readonly attention: readonly Fractal.Attention[]
  readonly onOpen: MissionDetailViewProps['onOpen']
  readonly onJump: (step: string) => void
  readonly offers: readonly MissionActionOffer[]
  readonly actions: MissionActions
}) => {
  const gate = attention.find(item => item.attention_kind === 'human-gate' || item.attention_kind === 'person-step') ?? attention[0]
  const youStep = plan.steps.find(step => step.word === 'needs_you')
  const stuck = presentation.stuck === null ? undefined : plan.steps.find(step => step.id === presentation.stuck!.step)
  const failed = presentation.word === 'failed' || stuck?.word === 'failed'
  if (presentation.word === 'decision' || mission.must_act === 'you' || youStep !== undefined) {
    return <div role="status" data-mission-banner="needs-you" {...stylex.props(styles.banner)}>
      <strong {...stylex.props(styles.bannerLead)}>Needs you</strong>
      <span {...stylex.props(styles.bannerText)}>{gate?.title ?? (youStep === undefined ? 'This mission waits on your answer.' : `${youStep.title} waits on you.`)}</span>
      <span {...stylex.props(styles.bannerActions)}>
        {gate !== undefined ? <Aria.Button onPress={() => onOpen(gate.id)} {...stylex.props(styles.action, styles.primary)}>Decide</Aria.Button> : null}
        {youStep !== undefined ? <Aria.Button onPress={() => onJump(youStep.id)} {...stylex.props(styles.action)}>Jump to step</Aria.Button> : null}
      </span>
    </div>
  }
  if (stuck === undefined && !failed && presentation.word !== 'stalled' && presentation.word !== 'unstaffed') return null
  const agent = presentation.stuck?.agent
  // The one thing most likely to unstick it: retry the failed step, else bring its seat back.
  const lead = offers.find(offer => offer.id === 'retry-step') ?? offers.find(offer => offer.id === 'restart-agent')
  const why = failed
    ? `${stuck?.title ?? 'A step'} failed${(stuck?.attempts ?? 0) > 1 ? ` after ${stuck!.attempts} attempts` : ''}.`
    : stuck === undefined ? 'Nothing is moving.'
      : `${stuck.title} is stuck${stuck.blockedReason !== undefined ? `: ${stuck.blockedReason}` : stuck.note !== null ? `: ${stuck.note}` : '.'}${agent !== null && agent !== undefined && !(stuck.note ?? '').includes(agent.label) ? ` ${agent.label} is ${agent.state}.` : ''}`
  return <div role="status" data-mission-banner={failed ? 'failed' : 'stuck'} {...stylex.props(styles.banner, failed && styles.bannerFailed)}>
    <strong {...stylex.props(styles.bannerLead, failed && styles.bannerLeadFailed)}>{failed ? 'Failed' : 'Stuck'}</strong>
    <span {...stylex.props(styles.bannerText)}>{why}</span>
    <span {...stylex.props(styles.bannerActions)}>
      {lead !== undefined ? <MissionActionControl offer={lead} actions={actions} primary /> : null}
      {stuck !== undefined ? <Aria.Button onPress={() => onJump(stuck.id)} {...stylex.props(styles.action)}>Jump to step</Aria.Button> : null}
      {agent !== null && agent !== undefined ? <Aria.Button onPress={() => onOpen(agent.id)} {...stylex.props(styles.action)}>Open {agent.label}</Aria.Button> : null}
    </span>
  </div>
}

const STEP_TONE: Record<StepWord, MissionRowTone> = { needs_you: 'needs-you', failed: 'failed', working: 'working', done: 'settled', cancelled: 'settled', ready: 'waiting', waiting: 'waiting', pending: 'waiting' }
const STEP_LABEL: Record<StepWord, string> = { needs_you: 'Needs you', done: 'Done', working: 'Working', ready: 'Ready', waiting: 'Waiting', cancelled: 'Cancelled', failed: 'Failed', pending: 'Pending' }

const StepMark = ({ word, spinning }: { readonly word: StepWord; readonly spinning: boolean }) =>
  word === 'working' ? <span title="Working" {...stylex.props(styles.stepMark)}><LiveRing size="sm" spinning={spinning} /><span {...stylex.props(styles.srOnly)}>Working</span></span>
    : word === 'done' ? <span title="Done" {...stylex.props(styles.stepMark)}><span aria-hidden="true" {...stylex.props(styles.doneDot)} /><span {...stylex.props(styles.srOnly)}>Done</span></span>
      : <span {...stylex.props(styles.stepMark)}><MissionStateMark tone={STEP_TONE[word]} label={STEP_LABEL[word]} /></span>

/** `compact`: one of several parallel steps in a column; owner and duration move under the title. */
const StepRow = ({ step, now, expanded, onToggle, onOpen, visible, compact = false, offers = [], actions }: { readonly step: PlanStep; readonly now: number; readonly expanded: boolean; readonly onToggle: () => void; readonly onOpen: MissionDetailViewProps['onOpen']; readonly visible: boolean; readonly compact?: boolean; readonly offers?: readonly MissionActionOffer[]; readonly actions?: MissionActions }) => {
  const duration = stepDuration(step, now)
  // Progress and goal are Markdown as written; the blocked reason and note are plain text, escaped to render as written.
  const lines = [step.goal !== undefined && markdownSource(step.goal) !== step.title ? step.goal : undefined, step.progress,
    step.blockedReason === undefined ? undefined : inlineFromPlain(step.blockedReason), step.note === null ? undefined : inlineFromPlain(step.note)]
  const detail = lines.filter((line, index): line is MarkdownInline => line !== undefined && markdownSource(line) !== '' && lines.findIndex(other => other !== undefined && markdownSource(other) === markdownSource(line)) === index)
  return <li data-step-id={step.id} data-step-word={step.word} {...stylex.props(styles.step, step.word === 'needs_you' && styles.stepYou, step.word === 'failed' && styles.stepFailed)}>
    <div {...stylex.props(styles.stepLine, compact && styles.stepLineCompact)}>
      <StepMark word={step.word} spinning={visible} />
      <Aria.Button aria-expanded={detail.length > 0 ? expanded : undefined} isDisabled={detail.length === 0} onPress={onToggle} {...stylex.props(styles.stepTitle)}>
        <span {...stylex.props(styles.ellipsis)}>{step.title}</span>
      </Aria.Button>
      {compact ? <span {...stylex.props(styles.lineBreak)} /> : null}
      {step.owner.kind === 'agent' && step.owner.agent !== undefined
        ? <Aria.Button onPress={event => onOpen(step.owner.agent!, event.metaKey || event.ctrlKey ? { background: true } : undefined)} {...stylex.props(styles.owner, styles.ownerButton, compact && styles.ownerCompact)}>{step.owner.label}</Aria.Button>
        : <span {...stylex.props(styles.owner, step.owner.kind === 'you' && styles.ownerYou, compact && styles.ownerCompact)}>{step.owner.label}</span>}
      <span {...stylex.props(styles.duration)}>{duration === undefined ? '' : `${formatDuration(duration, { maxUnits: 1 })}${step.word === 'working' ? '…' : ''}`}</span>
    </div>
    {step.attempts > 1 ? <div {...stylex.props(styles.stepSub)}>attempt {step.attempts}</div> : null}
    {offers.length > 0 && actions !== undefined ? <div {...stylex.props(styles.stepActions)}>{offers.map(offer => <MissionActionControl key={offer.action} offer={offer} actions={actions} />)}</div> : null}
    {expanded && detail.length > 0 ? <div {...stylex.props(styles.output)}>{detail.map((line, index) => <p key={index} {...stylex.props(styles.outputLine)}><InlineMarkdown text={line} /></p>)}</div> : null}
  </li>
}

/** The plan at a glance: one column per level, a dot per step, the dependency edges between them. Pressing a dot jumps to its step. */
const MiniGraph = ({ plan, onJump }: { readonly plan: MissionPlan; readonly onJump: (step: string) => void }) => {
  const dx = 28
  const dy = 16
  const tallest = Math.max(...plan.levels.map(level => level.length))
  const height = tallest * dy + 8
  const position = new Map<string, { x: number; y: number }>()
  plan.levels.forEach((level, depth) => level.forEach((step, index) => position.set(step.path, { x: 10 + depth * dx, y: height / 2 + (index - (level.length - 1) / 2) * dy })))
  const width = 20 + (plan.levels.length - 1) * dx
  return <svg role="group" aria-label="Dependency graph" width={width} height={height} viewBox={`0 0 ${width} ${height}`} {...stylex.props(styles.graph)}>
    {plan.edges.map(edge => {
      const from = position.get(edge.from)
      const to = position.get(edge.to)
      if (from === undefined || to === undefined) return null
      const mid = (from.x + to.x) / 2
      return <path key={`${edge.from}->${edge.to}`} d={`M${from.x} ${from.y} C${mid} ${from.y} ${mid} ${to.y} ${to.x} ${to.y}`} fill="none" stroke="currentColor" strokeOpacity={0.3} strokeWidth={1.25} />
    })}
    {plan.steps.map(step => {
      const at = position.get(step.path)!
      const fill = step.word === 'needs_you' ? tone.attention : step.word === 'failed' ? tone.dangerFg : step.word === 'done' || step.word === 'working' ? 'currentColor' : 'none'
      const stroke = step.word === 'needs_you' ? tone.attention : step.word === 'failed' ? tone.dangerFg : 'currentColor'
      return <g key={step.id} role="button" tabIndex={-1} aria-label={`${step.title}, ${STEP_LABEL[step.word]}`} onClick={() => onJump(step.id)} style={{ cursor: 'pointer' }}>
        <title>{`${step.title} · ${STEP_LABEL[step.word]}`}</title>
        <circle cx={at.x} cy={at.y} r={4.5} fill={fill} stroke={stroke} strokeOpacity={step.word === 'done' || fill !== 'none' ? 1 : 0.6} strokeWidth={1.25} fillOpacity={step.word === 'done' ? 0.55 : 1} />
      </g>
    })}
  </svg>
}

const ACTIVITY_LIMIT = 12
const Activity = ({ entries, now, onJump }: { readonly entries: readonly ActivityEntry[]; readonly now: number; readonly onJump: (step: string) => void }) => {
  const [all, setAll] = React.useState(false)
  if (entries.length === 0) return null
  const shown = all ? entries : entries.slice(0, ACTIVITY_LIMIT)
  return <section aria-labelledby="mission-activity" {...stylex.props(styles.section)}>
    <h2 id="mission-activity" {...stylex.props(styles.h2)}>Activity</h2>
    <ol {...stylex.props(styles.timeline)}>
      {shown.map((entry, index) => <li key={index} data-activity={entry.kind} {...stylex.props(styles.event)}>
        <span aria-hidden="true" {...stylex.props(styles.eventDot, entry.kind === 'failed' && styles.eventFailed, entry.kind === 'gate' && styles.eventYou, entry.kind === 'completed' && styles.eventDone)} />
        {entry.step !== undefined ? <Aria.Button onPress={() => onJump(entry.step!)} {...stylex.props(styles.eventText, styles.eventButton)}>{entry.text}</Aria.Button> : <span {...stylex.props(styles.eventText)}>{entry.text}</span>}
        <time dateTime={new Date(entry.at).toISOString()} title={new Date(entry.at).toLocaleString()} {...stylex.props(styles.soft, styles.eventTime)}>{compactAge(entry.at, now)} ago</time>
      </li>)}
    </ol>
    {entries.length > ACTIVITY_LIMIT ? <Aria.Button onPress={() => setAll(!all)} {...stylex.props(styles.more)}>{all ? 'Show fewer' : `Show all ${entries.length}`}</Aria.Button> : null}
  </section>
}

const LinkedWork = ({ mission, attention, onOpen }: { readonly mission: Fractal.Mission; readonly attention: readonly Fractal.Attention[]; readonly onOpen: MissionDetailViewProps['onOpen'] }) => {
  const resources = mission.resources ?? []
  if (resources.length === 0 && attention.length === 0) return null
  return <section aria-labelledby="mission-linked" {...stylex.props(styles.section)}>
    <h2 id="mission-linked" {...stylex.props(styles.h2)}>Linked work</h2>
    <ul {...stylex.props(styles.plainList)}>
      {attention.map(item => <li key={item.id}><Aria.Button data-open-ref={item.id} onPress={event => onOpen(item.id, event.metaKey || event.ctrlKey ? { background: true } : undefined)} {...stylex.props(styles.linkRow)}>
        <span {...stylex.props(styles.kind)}>Alert</span><span {...stylex.props(styles.grow, styles.ellipsis)}>{item.title}</span><span {...stylex.props(styles.soft)}>{item.priority}</span>
      </Aria.Button></li>)}
      {resources.map(resource => {
        const ref = resource.subject as string
        const label = resource.uri !== undefined && /\/pull\/\d+/.test(resource.uri) ? 'PR' : resource.kind === 'uri' ? 'Link' : 'Resource'
        return <li key={`${resource.name}:${ref}`}><Aria.Button data-open-ref={ref} onPress={event => onOpen(ref, event.metaKey || event.ctrlKey ? { background: true } : undefined)} {...stylex.props(styles.linkRow)}>
          <span {...stylex.props(styles.kind)}>{label}</span><span {...stylex.props(styles.grow, styles.ellipsis)}>{resource.name}</span>
          <span {...stylex.props(styles.soft, styles.ellipsis)}>{resource.resolution === 'unresolved' ? 'unresolved' : resource.uri ?? ''}</span>
        </Aria.Button></li>
      })}
    </ul>
  </section>
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', minHeight: 0, height: '100%', overflowY: 'auto', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  notice: { padding: s.lg },
  missing: { color: ink.fgSoft, fontSize: t.metaSize },
  header: { display: 'flex', flexDirection: 'column', gap: s.sm, paddingInline: s.xl, paddingBlock: s.lg, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  title: { margin: 0, fontSize: '16px', lineHeight: 1.3, fontWeight: 600, overflowWrap: 'anywhere' },
  meta: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: s.lg, rowGap: s.xs, color: ink.fgSoft, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  metaItem: { display: 'inline-flex', alignItems: 'center', gap: s.xs },
  soft: { color: ink.fgSoft, fontSize: t.denseSize },
  stack: { display: 'inline-flex' },
  avatar: { display: 'inline-grid', placeItems: 'center', flexShrink: 0, width: '18px', height: '18px', borderRadius: r.full, backgroundColor: surface.rowActive, color: ink.fgSoft, fontSize: '9px', fontWeight: 600 },
  stacked: { marginInlineStart: '-5px', borderWidth: '2px', borderStyle: 'solid', borderColor: surface.canvas, ':first-child': { marginInlineStart: 0 } },
  banner: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.md, marginBlockStart: s.xs, paddingInline: s.md, paddingBlock: s.sm, borderRadius: r.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: `color-mix(in srgb, ${tone.attention} 40%, transparent)`, backgroundColor: `color-mix(in srgb, ${tone.attention} 12%, transparent)` },
  bannerFailed: { borderColor: `color-mix(in srgb, ${tone.dangerFg} 40%, transparent)`, backgroundColor: `color-mix(in srgb, ${tone.dangerFg} 12%, transparent)` },
  bannerLead: { color: tone.attention, fontWeight: 600 },
  bannerLeadFailed: { color: tone.dangerFg },
  bannerText: { flex: '1 1 16rem', minWidth: 0 },
  bannerActions: { display: 'inline-flex', gap: s.sm, marginInlineStart: 'auto' },
  body: { display: 'flex', flexDirection: 'column', gap: s.xl, paddingInline: s.xl, paddingBlock: s.lg, maxWidth: '880px' },
  section: { display: 'flex', flexDirection: 'column', gap: s.sm },
  h2: { margin: 0, color: ink.fgMuted, fontSize: t.denseSize, fontWeight: 600, letterSpacing: '0.02em' },
  graph: { display: 'block', maxWidth: '100%', color: ink.fgSoft, marginBlockEnd: s.xs },
  stepper: { margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column' },
  level: { listStyle: 'none' },
  levelSteps: { margin: 0, padding: 0, listStyle: 'none', display: 'grid', gridTemplateColumns: 'minmax(0, 1fr)' },
  parallel: { gridTemplateColumns: 'repeat(auto-fit, minmax(14rem, 1fr))', columnGap: s.md },
  step: { display: 'flex', flexDirection: 'column', minWidth: 0, borderRadius: r.sm, paddingInline: s.xs, scrollMarginBlock: s.xl },
  stepYou: { backgroundColor: `color-mix(in srgb, ${tone.attention} 8%, transparent)` },
  stepFailed: { backgroundColor: `color-mix(in srgb, ${tone.dangerFg} 8%, transparent)` },
  stepLine: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0, minHeight: g.controlMd },
  stepLineCompact: { flexWrap: 'wrap', rowGap: 0, paddingBlock: s.xs },
  lineBreak: { flexBasis: '100%', height: 0, paddingInlineStart: `calc(${g.statusSm} + ${s.sm})` },
  stepMark: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, width: g.statusSm, height: g.statusSm },
  doneDot: { display: 'block', width: '7px', height: '7px', borderRadius: r.full, backgroundColor: ink.fgSoft },
  srOnly: { position: 'absolute', width: '1px', height: '1px', overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap' },
  stepTitle: { display: 'flex', flex: '1 1 0', minWidth: 0, margin: 0, padding: 0, borderWidth: 0, backgroundColor: 'transparent', color: 'inherit', fontFamily: 'inherit', fontSize: 'inherit', textAlign: 'start', cursor: { default: 'pointer', ':is([data-disabled])': 'default' }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted } },
  ellipsis: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  owner: { flexShrink: 1, minWidth: 0, maxWidth: '40%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fgSoft, fontSize: t.denseSize },
  ownerYou: { color: tone.attention },
  ownerCompact: { flex: '1 1 0', maxWidth: 'none', marginInlineStart: g.statusSm, textAlign: 'start' },
  ownerButton: { margin: 0, padding: 0, paddingInline: s.xs, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', fontFamily: 'inherit', cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, color: ink.fg } },
  duration: { flexShrink: 0, minWidth: '3rem', textAlign: 'end', color: ink.fgMuted, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  stepActions: { display: 'flex', flexWrap: 'wrap', gap: s.sm, paddingInlineStart: `calc(${g.statusSm} + ${s.sm})`, paddingBlockEnd: s.xs },
  stepSub: { paddingInlineStart: `calc(${g.statusSm} + ${s.sm})`, color: ink.fgMuted, fontSize: t.denseSize },
  output: { marginInlineStart: `calc(${g.statusSm} + ${s.sm})`, marginBlockEnd: s.sm, paddingInline: s.sm, paddingBlock: s.xs, borderRadius: r.sm, backgroundColor: surface.rowHover, color: ink.fgSoft, fontSize: t.metaSize },
  outputLine: { margin: 0, overflowWrap: 'anywhere' },
  plainList: { margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column' },
  linkRow: { display: 'flex', alignItems: 'center', gap: s.sm, width: '100%', minHeight: g.controlMd, paddingInline: s.xs, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fg, fontFamily: 'inherit', fontSize: t.metaSize, textAlign: 'start', cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted } },
  grow: { flex: '1 1 0', minWidth: 0 },
  state: { flexShrink: 0 },
  kind: { flexShrink: 0, minWidth: '4rem', color: ink.fgMuted, fontSize: t.denseSize },
  timeline: { margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column', borderInlineStartWidth: g.hairline, borderInlineStartStyle: 'solid', borderInlineStartColor: border.border, marginInlineStart: '3px' },
  event: { position: 'relative', display: 'flex', alignItems: 'baseline', gap: s.sm, paddingInlineStart: s.md, minHeight: g.controlSm },
  eventDot: { position: 'absolute', insetInlineStart: '-4px', top: '7px', width: '7px', height: '7px', borderRadius: r.full, backgroundColor: border.borderStrong },
  eventDone: { backgroundColor: ink.fgSoft },
  eventFailed: { backgroundColor: tone.dangerFg },
  eventYou: { backgroundColor: tone.attention },
  eventText: { flex: '1 1 0', minWidth: 0, fontSize: t.metaSize, color: ink.fg },
  eventButton: { margin: 0, padding: 0, borderWidth: 0, backgroundColor: 'transparent', fontFamily: 'inherit', textAlign: 'start', cursor: 'pointer', ':hover': { textDecorationLine: 'underline' } },
  eventTime: { flexShrink: 0, fontVariantNumeric: 'tabular-nums' },
  more: { alignSelf: 'flex-start', padding: 0, borderWidth: 0, backgroundColor: 'transparent', color: ink.fgSoft, fontFamily: 'inherit', fontSize: t.denseSize, cursor: 'pointer', ':hover': { color: ink.fg } },
  actions: { display: 'flex', flexWrap: 'wrap', gap: s.sm },
  action: { height: g.controlSm, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.control, backgroundColor: 'transparent', color: ink.fg, fontFamily: 'inherit', fontSize: t.denseSize, cursor: { default: 'pointer', ':is([data-disabled])': 'not-allowed' }, opacity: { default: 1, ':is([data-disabled])': 0.5 }, ':hover': { backgroundColor: surface.rowHover }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted } },
  primary: { borderColor: tone.attention, backgroundColor: tone.attention, color: '#1a1205', ':hover': { backgroundColor: tone.attention } },
})
