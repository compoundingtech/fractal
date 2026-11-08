import { inlineFromPlain, markdownSource, type MarkdownInline } from '@fractal/client/markdown'
import { useAtomValue } from '@effect/atom-react'
import type * as Fractal from '@fractal/client/schema'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { InlineMarkdown } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/Markdown.tsx'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t, interactionVars } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useDataSource, useGrants } from '../../data/react.tsx'
import { GithubHoverLink } from '../../plugins/github/card/GithubCard.tsx'
import { githubSubjectRef, parseGithubUrl } from '../../plugins/github/refs.ts'
import { MissionActionControl, useMissionActions, type MissionActions } from '../MissionActionControl.tsx'
import { missionActionOffers } from '../missionActions.ts'
import { runningMs, span, tokenCount, usd, waitedMs, type DetailStep, type MissionDetailModel } from './detailModel.ts'
import { useMissionFocus } from './focus.ts'
import { inspectedStep } from './linking.ts'
import { AGENT_LABEL, AgentRing, openGesture, STATE_LABEL, StepMark, type OpenRef } from './parts.tsx'
import { clock } from './traceScale.ts'

// The inspector: the step under the pointer, else the picked one, else the one most worth a look. It follows hover
// live (hovering a span or a node shows that step here) and snaps back to the picked step when the hover leaves. Below
// the step, the mission's own linked work. The same body fills the pane's side panel and, when no panel shows, the
// space under the trace.

export type SpendRead = 'known' | 'loading' | 'unavailable'

/** The offered actions, with their runs shared by every view of this mission. */
export const useDetailActions = (mission: Fractal.Mission, onOpen: OpenRef): MissionActions => {
  const source = useDataSource()
  const grants = useGrants()
  return useMissionActions({ port: source.missionActions, mission, readOnly: grants.actions !== 'granted', onOpen: ref => onOpen(ref) })
}

export const agentLabelOf = (model: MissionDetailModel) => (id: string | undefined): string | undefined =>
  id === undefined ? undefined : model.agents.find(agent => agent.id === id)?.label ?? id.split('/').pop()

export const MissionInspector = ({ missionRef, mission, model, spend, onOpen, actions }: {
  readonly missionRef: string
  readonly mission: Fractal.Mission
  readonly model: MissionDetailModel
  readonly spend: SpendRead
  readonly onOpen: OpenRef
  readonly actions: MissionActions
}) => {
  const { focus } = useMissionFocus(missionRef)
  const inspected = inspectedStep(model, focus)
  const step = model.steps.find(each => each.id === inspected.id)
  return <div data-inspector="" {...stylex.props(styles.root)}>
    {step === undefined
      ? <p {...stylex.props(styles.soft)}>Fractal did not send this mission's steps.</p>
      : <StepDetails key={step.id} step={step} source={inspected.source} mission={mission} model={model} spend={spend} onOpen={onOpen} actions={actions} />}
    <MissionLinks mission={mission} model={model} onOpen={onOpen} />
  </div>
}

const OUTCOME = { done: 'done', failed: 'failed', running: 'running', cancelled: 'cancelled' } as const

const StepDetails = ({ step, source, mission, model, spend, onOpen, actions }: {
  readonly step: DetailStep
  readonly source: 'hovered' | 'selected' | 'default'
  readonly mission: Fractal.Mission
  readonly model: MissionDetailModel
  readonly spend: SpendRead
  readonly onOpen: OpenRef
  readonly actions: MissionActions
}) => {
  const agent = model.agents.find(each => each.id === step.agent)
  const ran = runningMs(step, model.now)
  const waited = waitedMs(step, model.now)
  const onYou = waitedMs(step, model.now, 'you')
  const problems = model.problems.filter(problem => problem.step === step.id)
  const offers = missionActionOffers(mission).filter(offer => offer.target === step.id)
  const links = (mission.resources ?? []).filter(resource => resource.name === step.path)
  // Progress and goal are Markdown as written; the blocked reason and note are plain text, escaped to render as written.
  const lines = [step.goal !== undefined && markdownSource(step.goal) !== step.title ? step.goal : undefined, step.progress,
    step.blockedReason === undefined ? undefined : inlineFromPlain(step.blockedReason), step.note === null ? undefined : inlineFromPlain(step.note)]
  const notes = lines.filter((line, index): line is MarkdownInline => line !== undefined && markdownSource(line) !== '' && lines.findIndex(other => other !== undefined && markdownSource(other) === markdownSource(line)) === index)
  const asked = step.waits.some(wait => wait.kind === 'you') || step.word === 'needs_you'
  return <section aria-label={`Step: ${step.title}`} data-inspector-step={step.id} data-inspector-source={source} {...stylex.props(styles.section)}>
    <header {...stylex.props(styles.header)}>
      <StepMark state={step.state} size={12} />
      <h2 {...stylex.props(styles.title)}>{step.title}</h2>
      <span {...stylex.props(styles.state, step.state === 'waiting-you' && styles.you, step.state === 'failed' && styles.bad)}>{STATE_LABEL[step.state]}</span>
    </header>
    {source === 'hovered' ? <p data-inspector-following="" {...stylex.props(styles.following)}>Following the pointer; it returns to the picked step.</p> : null}
    <dl {...stylex.props(styles.facts)}>
      <dt {...stylex.props(styles.term)}>Who</dt>
      <dd {...stylex.props(styles.value)}>
        {step.agent === undefined ? step.owner.label
          : <Aria.Button data-open-ref={step.agent} onPress={event => onOpen(step.agent!, openGesture(event))} {...stylex.props(styles.agent)}>
            <AgentRing state={agent?.state ?? 'unknown'} /><span>{agent?.label ?? step.agent.split('/').pop()}</span><span {...stylex.props(styles.soft)}>{AGENT_LABEL[agent?.state ?? 'unknown']}</span>
          </Aria.Button>}
      </dd>
      <dt {...stylex.props(styles.term)}>Time</dt>
      <dd {...stylex.props(styles.value)}>
        {ran > 0 ? `ran ${span(ran)}` : 'not started'}
        {waited > onYou ? ` · waited ${span(waited - onYou)}` : ''}
        {onYou > 0 ? <> · <span {...stylex.props(styles.you)}>waited on you {span(onYou)}</span></> : null}
        {step.expected === undefined ? '' : ` · usually ${span(step.expected.lowMs)}–${span(step.expected.highMs)}`}
      </dd>
      <dt {...stylex.props(styles.term)}>Spend</dt>
      <dd data-inspector-spend="" {...stylex.props(styles.value)}>
        {step.spend !== undefined ? `${usd(step.spend.costUsd)} · ${tokenCount(step.spend.tokens)} tokens`
          : spend === 'loading' ? 'Loading…' : spend === 'unavailable' ? 'Not reported here' : 'No usage recorded for this step'}
      </dd>
    </dl>
    {notes.length > 0 ? <div {...stylex.props(styles.notes)}>{notes.map((line, index) => <p key={index} {...stylex.props(styles.note)}><InlineMarkdown text={line} /></p>)}</div> : null}
    {step.attempts.length > 0 || step.attemptCount > 1 ? <div {...stylex.props(styles.block)}>
      <h3 {...stylex.props(styles.h3)}>Attempts</h3>
      <ol {...stylex.props(styles.list)}>
        {step.attempts.map(attempt => <li key={attempt.n} data-inspector-attempt={attempt.outcome} {...stylex.props(styles.item)}>
          <span {...stylex.props(styles.itemLine)}>
            <span {...stylex.props(styles.grow, attempt.outcome === 'failed' && styles.bad)}>Attempt {attempt.n}: {OUTCOME[attempt.outcome]}</span>
            <span {...stylex.props(styles.soft)}>{span((attempt.end ?? model.now) - attempt.start)} · {clock(attempt.start)}</span>
          </span>
          {attempt.reason === undefined ? null : <span {...stylex.props(styles.soft)}>{attempt.reason}</span>}
        </li>)}
      </ol>
      {step.historyComplete ? null : <p {...stylex.props(styles.soft)}>Fractal serves only the latest of {step.attemptCount} attempts so far.</p>}
    </div> : null}
    {problems.length > 0 ? <div {...stylex.props(styles.block)}>
      <h3 {...stylex.props(styles.h3)}>Problems</h3>
      <ul {...stylex.props(styles.list)}>
        {problems.map((problem, index) => <li key={index} {...stylex.props(styles.item)}>
          <span {...stylex.props(styles.itemLine)}><span {...stylex.props(styles.grow, problem.kind === 'failure' && styles.bad, problem.kind === 'wait-you' && styles.you)}>{problem.text}</span><span {...stylex.props(styles.soft)}>{clock(problem.at)}</span></span>
        </li>)}
      </ul>
    </div> : null}
    {asked || source === 'selected' ? <Answers stepId={step.id} /> : null}
    {links.length > 0 ? <div {...stylex.props(styles.block)}>
      <h3 {...stylex.props(styles.h3)}>Linked work</h3>
      <ul {...stylex.props(styles.list)}>{links.map(resource => <li key={`${resource.name}:${resource.subject}`}><LinkRow resource={resource} now={model.now} onOpen={onOpen} /></li>)}</ul>
    </div> : null}
    {offers.length > 0 ? <div {...stylex.props(styles.actions)}>{offers.map(offer => <MissionActionControl key={offer.action} offer={offer} actions={actions} />)}</div> : null}
  </section>
}

/** The person's answers recorded on the step's work, when Fractal serves them. Read only for a picked step or one that asked. */
const Answers = ({ stepId }: { readonly stepId: string }) => {
  const feed = useAtomValue(useDataSource().subjectReads.work.feed(stepId))
  const answers = feed._tag === 'Observed' ? feed.value.person_answers ?? [] : []
  if (answers.length === 0) return null
  return <div data-inspector-answers="" {...stylex.props(styles.block)}>
    <h3 {...stylex.props(styles.h3)}>Your answers</h3>
    <ul {...stylex.props(styles.list)}>
      {answers.map(answer => <li key={answer.ask} {...stylex.props(styles.item)}>
        <span {...stylex.props(styles.itemLine)}><span {...stylex.props(styles.grow)}>{answer.summary}</span><span {...stylex.props(styles.soft)}>{clock(answer.answered_at_unix_ms)}</span></span>
        {answer.status === 'cancelled' ? <span {...stylex.props(styles.soft)}>Withdrawn</span> : null}
      </li>)}
    </ul>
  </div>
}

type Resource = NonNullable<Fractal.Mission['resources']>[number]

/** A declared resource: a GitHub link carries its hover card and opens as its own tab; anything else opens its resource. */
const LinkRow = ({ resource, now, onOpen }: { readonly resource: Resource; readonly now: number; readonly onOpen: OpenRef }) => {
  const uri = resource.uri ?? undefined
  const github = uri === undefined ? undefined : parseGithubUrl(uri)
  const ref = (uri === undefined ? undefined : githubSubjectRef(uri)) ?? (resource.subject as string)
  const kind = github === undefined ? (resource.kind === 'uri' ? 'Link' : 'Resource') : github.kind === 'pull-request' ? 'PR' : github.kind === 'issue' ? 'Issue' : github.kind === 'run' ? 'Run' : 'GitHub'
  const row = <Aria.Button data-open-ref={ref} data-linked-work={kind} onPress={event => onOpen(ref, openGesture(event))} {...stylex.props(styles.link)}>
    <span {...stylex.props(styles.kind)}>{kind}</span>
    <span {...stylex.props(styles.grow, styles.ellipsis)}>{resource.name}</span>
    <span {...stylex.props(styles.soft, styles.ellipsis, styles.uri)}>{resource.resolution === 'unresolved' ? 'unresolved' : uri ?? ''}</span>
  </Aria.Button>
  return github === undefined ? row : <GithubHoverLink target={github} now={now}>{row}</GithubHoverLink>
}

/** What the mission declares that no step names, and its open alerts. */
const MissionLinks = ({ mission, model, onOpen }: { readonly mission: Fractal.Mission; readonly model: MissionDetailModel; readonly onOpen: OpenRef }) => {
  const paths = new Set(model.steps.map(step => step.path))
  const resources = (mission.resources ?? []).filter(resource => !paths.has(resource.name))
  if (resources.length === 0) return null
  return <section aria-label="Mission links" data-mission-links="" {...stylex.props(styles.section, styles.missionLinks)}>
    <h3 {...stylex.props(styles.h3)}>Mission links</h3>
    <ul {...stylex.props(styles.list)}>{resources.map(resource => <li key={`${resource.name}:${resource.subject}`}><LinkRow resource={resource} now={model.now} onOpen={onOpen} /></li>)}</ul>
  </section>
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', gap: s.lg, minWidth: 0, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  section: { display: 'flex', flexDirection: 'column', gap: s.md, minWidth: 0 },
  missionLinks: { paddingBlockStart: s.md, borderBlockStartWidth: g.hairline, borderBlockStartStyle: 'solid', borderBlockStartColor: border.border },
  header: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0 },
  title: { flex: '1 1 auto', minWidth: 0, margin: 0, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: 600, overflowWrap: 'anywhere' },
  state: { flexShrink: 0, color: ink.fgMuted, fontSize: t.denseSize },
  following: { margin: 0, color: ink.fgMuted, fontSize: t.denseSize },
  facts: { display: 'grid', gridTemplateColumns: 'auto minmax(0, 1fr)', columnGap: s.md, rowGap: s.xs, margin: 0 },
  term: { color: ink.fgMuted, fontSize: t.denseSize, lineHeight: t.metaLeading },
  value: { margin: 0, minWidth: 0, overflowWrap: 'anywhere', fontVariantNumeric: 'tabular-nums' },
  agent: { display: 'inline-flex', alignItems: 'center', gap: s.sm, maxWidth: '100%', margin: 0, paddingInline: s.xs, marginInlineStart: `calc(-1 * ${s.xs})`, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fg, fontFamily: 'inherit', fontSize: 'inherit', cursor: 'pointer',
    ':hover': { backgroundColor: surface.rowHover }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing } },
  notes: { display: 'flex', flexDirection: 'column', gap: s.xs, paddingInline: s.sm, paddingBlock: s.xs, borderRadius: r.sm, backgroundColor: surface.rowHover, color: ink.fgSoft },
  note: { margin: 0, overflowWrap: 'anywhere' },
  block: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0 },
  h3: { margin: 0, color: ink.fgMuted, fontSize: t.denseSize, fontWeight: 600, letterSpacing: '0.02em' },
  list: { display: 'flex', flexDirection: 'column', gap: s.xs, margin: 0, padding: 0, listStyle: 'none' },
  item: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  itemLine: { display: 'flex', alignItems: 'baseline', gap: s.sm, minWidth: 0 },
  grow: { flex: '1 1 auto', minWidth: 0, overflowWrap: 'anywhere' },
  soft: { color: ink.fgMuted, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  you: { color: `color-mix(in srgb, ${tone.attention} 80%, ${ink.fg})` },
  bad: { color: `color-mix(in srgb, ${tone.dangerFg} 80%, ${ink.fg})` },
  actions: { display: 'flex', flexWrap: 'wrap', gap: s.sm },
  link: { display: 'flex', alignItems: 'center', gap: s.sm, width: '100%', minHeight: g.controlMd, paddingInline: s.xs, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fg, fontFamily: 'inherit', fontSize: t.metaSize, textAlign: 'start', cursor: 'pointer',
    ':hover': { backgroundColor: surface.rowHover }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing } },
  kind: { flexShrink: 0, minWidth: '3rem', color: ink.fgMuted, fontSize: t.denseSize },
  ellipsis: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  uri: { flex: '0 1 45%', minWidth: 0 },
})
