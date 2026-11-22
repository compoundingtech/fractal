import type * as Fractal from '@fractal/client/schema'
import { MissionProgressMark, MissionStateMark, compactAge } from '@fractal/ui/assistant-ui/shell'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t, interactionVars } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { MissionActionControl, type MissionActions } from '../MissionActionControl.tsx'
import { missionActionOffers } from '../missionActions.ts'
import { missionRowData } from '../rowData.ts'
import { span, stepEnd, tokenCount, usd, type MissionDetailModel } from './detailModel.ts'
import type { MissionFocusControls } from './focus.ts'
import type { SpendRead } from './Inspector.tsx'
import { AGENT_LABEL, AgentRing, openGesture, SrOnly, type OpenRef } from './parts.tsx'

// How it is going, always on top: state, progress, time spent against what is left, spend, then what runs now, what
// needs the person or is blocked, and what comes next. An amber banner names why the person must act (red only for a
// failure) with the one action. Below it, who is on the mission.

export const Overview = ({ mission, model, attention, stale, spend, visible, controls, actions, onOpen }: {
  readonly mission: Fractal.Mission
  readonly model: MissionDetailModel
  readonly attention: readonly Fractal.Attention[]
  readonly stale: boolean
  readonly spend: SpendRead
  readonly visible: boolean
  readonly controls: MissionFocusControls
  readonly actions: MissionActions
  readonly onOpen: OpenRef
}) => {
  const data = missionRowData({ mission, presentation: model.presentation })
  const titleOf = (id: string) => model.steps.find(step => step.id === id)?.title ?? id
  const settled = model.run?.settled ?? false
  const lastEnd = Math.max(model.startedAt, ...model.steps.flatMap(step => stepEnd(step, model.now) ?? []))
  const elapsed = (settled ? lastEnd : model.now) - model.startedAt
  const eta = model.run?.eta
  const facts: ReadonlyArray<{ readonly label: string; readonly steps: readonly string[]; readonly tone?: 'you' | 'bad' }> = [
    { label: 'Needs you', steps: model.needsYou, tone: 'you' },
    { label: 'Failed', steps: model.failed, tone: 'bad' },
    { label: 'Running', steps: model.running },
    { label: 'Blocked', steps: model.blocked },
    { label: 'Next', steps: model.next === undefined ? [] : [model.next] },
  ]
  return <header data-mission-overview="" {...stylex.props(styles.header)}>
    <h1 {...stylex.props(styles.title)}>{model.title}</h1>
    <div {...stylex.props(styles.meta)}>
      <span {...stylex.props(styles.metaItem)}><MissionStateMark tone={data.tone} label={data.stateLabel} frozen={stale || !visible} /><span aria-hidden="true">{data.stateLabel}</span></span>
      {data.progress._tag === 'Known' ? <span {...stylex.props(styles.metaItem)}><MissionProgressMark progress={data.progress} retained={stale} /><span aria-hidden="true">steps</span></span> : null}
      <span data-mission-time="" {...stylex.props(styles.metaItem)}>
        {settled ? `took ${span(elapsed)}` : `${span(elapsed)} in`}
        {!settled && eta !== undefined ? ` · about ${span(eta.lowMs)}–${span(eta.highMs)} left` : ''}
      </span>
      <span data-mission-spend={spend} {...stylex.props(styles.metaItem)}>
        {model.spend !== undefined ? `${usd(model.spend.costUsd)} · ${tokenCount(model.spend.tokens)} tokens`
          : spend === 'loading' ? 'Spend loading…' : 'Spend not reported'}
      </span>
      <span {...stylex.props(styles.metaItem, styles.soft)}>{stale ? 'last observed ' : 'updated '}{compactAge(data.updatedAt, model.now)} ago</span>
    </div>
    <dl {...stylex.props(styles.facts)}>
      {facts.filter(fact => fact.steps.length > 0).map(fact => <div key={fact.label} data-mission-fact={fact.label} {...stylex.props(styles.fact)}>
        <dt {...stylex.props(styles.factLabel, fact.tone === 'you' && styles.you, fact.tone === 'bad' && styles.bad)}>{fact.label}</dt>
        <dd {...stylex.props(styles.factSteps)}>
          {fact.steps.map(id => <Aria.Button key={id} onPress={() => controls.select(id)} onHoverStart={() => controls.hover(id)} onHoverEnd={() => controls.hover(undefined)} {...stylex.props(styles.stepLink)}>{titleOf(id)}</Aria.Button>)}
        </dd>
      </div>)}
    </dl>
    <Banner mission={mission} model={model} attention={attention} controls={controls} actions={actions} onOpen={onOpen} />
    <RunActions mission={mission} model={model} actions={actions} />
  </header>
}

/** Offers on the run rather than on one step (cancel the run, restart a seat); a step's own offers live in the inspector. */
const RunActions = ({ mission, model, actions }: { readonly mission: Fractal.Mission; readonly model: MissionDetailModel; readonly actions: MissionActions }) => {
  const steps = new Set(model.steps.map(step => step.id))
  const offers = missionActionOffers(mission).filter(offer => !steps.has(offer.target))
  if (offers.length === 0) return null
  return <div role="group" aria-label="Mission actions" {...stylex.props(styles.runActions)}>
    {offers.map(offer => <MissionActionControl key={`${offer.id}:${offer.action}:${offer.target}`} offer={offer} actions={actions} />)}
  </div>
}

/** Amber when the person must act or the mission is stuck, red only for a failure; nothing while it moves. */
const Banner = ({ mission, model, attention, controls, actions, onOpen }: {
  readonly mission: Fractal.Mission
  readonly model: MissionDetailModel
  readonly attention: readonly Fractal.Attention[]
  readonly controls: MissionFocusControls
  readonly actions: MissionActions
  readonly onOpen: OpenRef
}) => {
  const presentation = model.presentation
  const gate = attention.find(item => item.attention_kind === 'human-gate' || item.attention_kind === 'person-step') ?? attention[0]
  const byId = (id: string | undefined) => id === undefined ? undefined : model.steps.find(step => step.id === id)
  const youStep = byId(model.needsYou[0])
  const failedStep = byId(model.failed[0])
  const stuck = presentation.stuck === null ? undefined : byId(presentation.stuck.step)
  const show = (id: string) => <Aria.Button onPress={() => controls.select(id)} {...stylex.props(styles.action)}>Show step</Aria.Button>
  if (presentation.word === 'decision' || mission.must_act === 'you' || youStep !== undefined) {
    return <div role="status" data-mission-banner="needs-you" {...stylex.props(styles.banner)}>
      <strong {...stylex.props(styles.bannerLead)}>Needs you</strong>
      <span {...stylex.props(styles.bannerText)}>{gate?.title ?? (youStep === undefined ? 'This mission waits on your answer.' : `${youStep.title} waits on you.`)}</span>
      <span {...stylex.props(styles.bannerActions)}>
        {gate !== undefined ? <Aria.Button onPress={event => onOpen(gate.id, openGesture(event))} {...stylex.props(styles.action, styles.primary)}>Decide</Aria.Button> : null}
        {youStep !== undefined ? show(youStep.id) : null}
      </span>
    </div>
  }
  const failed = failedStep !== undefined || presentation.word === 'failed'
  if (!failed && stuck === undefined && presentation.word !== 'stalled' && presentation.word !== 'unstaffed') return null
  const subject = failedStep ?? stuck
  const agent = presentation.stuck?.agent
  const offers = missionActionOffers(mission)
  // The one thing most likely to unstick it: retry the failed step, else bring its seat back.
  const lead = offers.find(offer => offer.id === 'retry-step' && (subject === undefined || offer.target === subject.id)) ?? offers.find(offer => offer.id === 'retry-step') ?? offers.find(offer => offer.id === 'restart-agent')
  const why = failed
    ? `${subject?.title ?? 'A step'} failed${(subject?.attemptCount ?? 0) > 1 ? ` after ${subject!.attemptCount} attempts` : ''}.`
    : stuck === undefined ? 'Nothing is moving.'
      : `${stuck.title} is stuck${stuck.blockedReason !== undefined ? `: ${stuck.blockedReason}` : stuck.note !== null ? `: ${stuck.note}` : '.'}${agent !== null && agent !== undefined && !(stuck.note ?? '').includes(agent.label) ? ` ${agent.label} is ${agent.state}.` : ''}`
  return <div role="status" data-mission-banner={failed ? 'failed' : 'stuck'} {...stylex.props(styles.banner, failed && styles.bannerFailed)}>
    <strong {...stylex.props(styles.bannerLead, failed && styles.bannerLeadFailed)}>{failed ? 'Failed' : 'Stuck'}</strong>
    <span {...stylex.props(styles.bannerText)}>{why}</span>
    <span {...stylex.props(styles.bannerActions)}>
      {lead !== undefined ? <MissionActionControl offer={lead} actions={actions} primary /> : null}
      {subject !== undefined ? show(subject.id) : null}
      {agent !== null && agent !== undefined ? <Aria.Button onPress={event => onOpen(agent.id, openGesture(event))} {...stylex.props(styles.action)}>Open {agent.label}</Aria.Button> : null}
    </span>
  </div>
}

/** Who is on the mission: a live state ring, the step each holds, one press to open the agent. */
export const AgentsStrip = ({ model, controls, onOpen }: { readonly model: MissionDetailModel; readonly controls: MissionFocusControls; readonly onOpen: OpenRef }) => {
  if (model.agents.length === 0) return null
  return <ul aria-label="Agents on this mission" {...stylex.props(styles.strip)}>
    {model.agents.map(agent => {
      const step = model.steps.find(each => each.id === agent.step)
      return <li key={agent.id} {...stylex.props(styles.chipItem)}>
        <Aria.Button data-agent-chip={agent.id}
          onPress={event => onOpen(agent.id, openGesture(event))}
          onHoverStart={() => { if (step !== undefined) controls.hover(step.id) }} onHoverEnd={() => controls.hover(undefined)}
          {...stylex.props(styles.chip)}>
          <AgentRing state={agent.state} />
          <span>{agent.label}<SrOnly>, {AGENT_LABEL[agent.state]}</SrOnly></span>
          {step === undefined ? null : <span {...stylex.props(styles.chipStep, agent.state === 'needs-you' && styles.you, agent.state === 'failed' && styles.bad)}>· {step.title}</span>}
        </Aria.Button>
      </li>
    })}
  </ul>
}

const styles = stylex.create({
  header: { display: 'flex', flexDirection: 'column', gap: s.sm, paddingInline: s.xl, paddingBlock: s.lg, minWidth: 0 },
  title: { margin: 0, fontSize: t.headingSize, lineHeight: t.headingLeading, fontWeight: 600, overflowWrap: 'anywhere' },
  meta: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: s.lg, rowGap: s.xs, color: ink.fgSoft, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  metaItem: { display: 'inline-flex', alignItems: 'center', gap: s.xs },
  soft: { color: ink.fgMuted },
  facts: { display: 'flex', flexWrap: 'wrap', columnGap: s.xl, rowGap: s.xs, margin: 0, fontSize: t.metaSize },
  fact: { display: 'flex', alignItems: 'baseline', gap: s.sm, minWidth: 0, maxWidth: '100%' },
  factLabel: { flexShrink: 0, color: ink.fgMuted, fontSize: t.denseSize },
  factSteps: { display: 'flex', flexWrap: 'wrap', gap: s.sm, margin: 0, minWidth: 0 },
  stepLink: { margin: 0, padding: 0, borderWidth: 0, backgroundColor: 'transparent', color: ink.fg, fontFamily: 'inherit', fontSize: 'inherit', textAlign: 'start', cursor: 'pointer', overflowWrap: 'anywhere',
    textDecorationLine: { default: 'none', ':hover': 'underline' }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing } },
  you: { color: `color-mix(in srgb, ${tone.attention} 80%, ${ink.fg})` },
  bad: { color: `color-mix(in srgb, ${tone.dangerFg} 80%, ${ink.fg})` },
  banner: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.md, marginBlockStart: s.xs, paddingInline: s.md, paddingBlock: s.sm, borderRadius: r.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: `color-mix(in srgb, ${tone.attention} 40%, transparent)`, backgroundColor: `color-mix(in srgb, ${tone.attention} 8%, transparent)` },
  bannerFailed: { borderColor: `color-mix(in srgb, ${tone.dangerFg} 40%, transparent)`, backgroundColor: `color-mix(in srgb, ${tone.dangerFg} 8%, transparent)` },
  bannerLead: { color: `color-mix(in srgb, ${tone.attention} 80%, ${ink.fg})`, fontWeight: 600 },
  bannerLeadFailed: { color: `color-mix(in srgb, ${tone.dangerFg} 80%, ${ink.fg})` },
  bannerText: { flex: '1 1 16rem', minWidth: 0, overflowWrap: 'anywhere' },
  bannerActions: { display: 'inline-flex', flexWrap: 'wrap', gap: s.sm, marginInlineStart: 'auto' },
  action: { display: 'inline-flex', alignItems: 'center', height: g.controlSm, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fg, fontFamily: 'inherit', fontSize: t.metaSize, cursor: 'pointer',
    ':hover': { backgroundColor: surface.rowHover }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing } },
  primary: { borderColor: `color-mix(in srgb, ${tone.attention} 60%, transparent)`, color: `color-mix(in srgb, ${tone.attention} 80%, ${ink.fg})`, fontWeight: 600 },
  runActions: { display: 'flex', flexWrap: 'wrap', alignItems: 'flex-start', gap: s.sm },
  strip: { display: 'flex', flexWrap: 'wrap', gap: s.sm, margin: 0, paddingInline: s.xl, paddingBlockEnd: s.md, listStyle: 'none' },
  chipItem: { minWidth: 0, maxWidth: '100%' },
  chip: { display: 'inline-flex', alignItems: 'center', gap: s.sm, maxWidth: '100%', height: g.controlSm, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.full, backgroundColor: 'transparent', color: ink.fg, fontFamily: 'inherit', fontSize: t.metaSize, cursor: 'pointer', whiteSpace: 'nowrap', overflow: 'hidden',
    ':hover': { backgroundColor: surface.rowHover }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing } },
  chipStep: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', color: ink.fgMuted },
})
