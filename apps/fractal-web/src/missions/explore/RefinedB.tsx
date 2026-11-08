// EXPLORATION ONLY: variant B refined after the second interview. Overview with spend and agents; Execution (replay
// scrubber, graph over trace with linked hover, brush and replay, then Problems and History) and Definition (the KDL,
// and where the TypeScript source will show) as the detail's tabs. One step card everywhere.
import * as React from 'react'

import { EntityCard, EntityCardHeader, EntityCardList, EntityCardListItem, EntityCardMeta, EntityCardMetaLine, EntityCardNote } from '../../../../../packages/fractal-ui/src/assistant-ui/entity/EntityCard.tsx'
import { GithubHoverLink } from '../../plugins/github/card/GithubCard.tsx'
import type { GithubRef } from '../../plugins/github/refs.ts'
import { LinkedDag, LinkedTrace, Replay, StepCardHost, useLinked, type Linked } from './linked.tsx'
import { clock, runningMs, span, stateAt, tokenCount, usd, type ExploreMission, type LinkedWork, type MissionAgent, type PlanStep } from './model.ts'
import { C, History, Mark, Overview, Problems, STATE_LABEL, Tabs } from './parts.tsx'

const totalUsage = (mission: ExploreMission) => mission.steps.reduce((sum, s) => ({ costUsd: sum.costUsd + (s.usage?.costUsd ?? 0), tokens: sum.tokens + (s.usage?.tokens ?? 0) }), { costUsd: 0, tokens: 0 })
const agentOf = (step: PlanStep) => step.attempts.at(-1)?.agent ?? 'unassigned'

/** The one step card: state, who, how long, attempts, spend, problems. Same body in the graph, the trace and the lists. */
export const StepCard = ({ mission, step, cursor }: { readonly mission: ExploreMission; readonly step: PlanStep; readonly cursor?: number | undefined }) => {
  const state = cursor === undefined ? step.state : stateAt(step, cursor, mission.now)
  const problems = mission.problems.filter(p => p.step === step.id)
  const waited = step.waits.reduce((sum, w) => sum + ((w.end ?? mission.now) - w.start), 0)
  return <EntityCard entity={{ kind: 'mission', id: `${mission.id}#${step.id}` }} testId="step-card">
    <EntityCardHeader title={step.title} kind="Step" glyph={<Mark state={state} size={12} />} summary={[STATE_LABEL[state], agentOf(step), cursor === undefined ? undefined : `at ${clock(cursor)}`]} />
    <EntityCardMeta>
      <EntityCardMetaLine icon="clock" label="Time">ran {span(runningMs(step, mission.now))}{waited > 0 ? `, waited ${span(waited)}` : ''}{step.expectedMs ? `, usually ${span(step.expectedMs)}` : ''}</EntityCardMetaLine>
      {step.usage !== undefined ? <EntityCardMetaLine icon="gauge" label="Spend">{usd(step.usage.costUsd)} · {tokenCount(step.usage.tokens)} tokens</EntityCardMetaLine> : null}
    </EntityCardMeta>
    {step.attempts.length > 0 ? <EntityCardList>
      {step.attempts.map(a => <EntityCardListItem key={a.n} title={`Attempt ${a.n}: ${a.outcome}`} trailing={span((a.end ?? mission.now) - a.start)} {...(a.reason === undefined ? {} : { detail: a.reason })} />)}
    </EntityCardList> : null}
    {problems.length > 0 ? <EntityCardNote>{problems.length} problem{problems.length === 1 ? '' : 's'}: {problems.at(-1)!.text}</EntityCardNote> : null}
  </EntityCard>
}

const agentTone: Record<MissionAgent['state'], string> = { working: C.fg, idle: C.muted, 'needs-you': C.you, failed: C.bad, stopped: C.muted }
const AgentRing = ({ state }: { readonly state: MissionAgent['state'] }) =>
  <svg aria-hidden="true" width={12} height={12} viewBox="0 0 12 12"><circle cx={6} cy={6} r={4.5} fill={state === 'needs-you' || state === 'failed' ? agentTone[state] : 'none'} stroke={agentTone[state]} strokeWidth={1.5} strokeDasharray={state === 'stopped' ? '2 2' : state === 'working' ? '20 8' : undefined} /></svg>

/** Who is on the mission now: a status ring, their step, one press to open the agent. */
const AgentsStrip = ({ mission, linked, onOpen }: { readonly mission: ExploreMission; readonly linked: Linked; readonly onOpen: (ref: string) => void }) =>
  <div role="list" aria-label="Agents on this mission" style={{ display: 'flex', flexWrap: 'wrap', gap: 6, padding: '8px 20px', borderBottom: `1px solid ${C.line}` }}>
    {(mission.agents ?? []).map(agent => {
      const step = mission.steps.find(s => s.id === agent.step)
      return <span role="listitem" key={agent.id}><button type="button" data-agent-chip={agent.id} onClick={() => onOpen(agent.id)}
        onPointerEnter={() => { if (step !== undefined) linked.hover(step.id) }} onPointerLeave={() => linked.hover(undefined)}
        style={{ display: 'inline-flex', alignItems: 'center', gap: 6, height: 24, padding: '0 8px', border: `1px solid ${C.line}`, borderRadius: 12, background: 'transparent', color: C.fg, font: 'inherit', fontSize: 12, cursor: 'pointer' }}>
        <AgentRing state={agent.state} />{agent.name}{step !== undefined ? <span style={{ color: agent.state === 'needs-you' ? C.you : agent.state === 'failed' ? C.bad : C.muted }}>· {step.title}</span> : null}
      </button></span>
    })}
  </div>

const githubRef = (work: LinkedWork): GithubRef | undefined =>
  work.owner === undefined || work.repo === undefined ? undefined
    : work.kind === 'pull-request' && work.number !== undefined ? { kind: 'pull-request', owner: work.owner, repo: work.repo, number: work.number }
      : work.kind === 'issue' && work.number !== undefined ? { kind: 'issue', owner: work.owner, repo: work.repo, number: work.number }
        : work.kind === 'run' && work.runId !== undefined ? { kind: 'run', owner: work.owner, repo: work.repo, run_id: work.runId } : undefined
const workWord: Record<LinkedWork['kind'], string> = { 'pull-request': 'PR', issue: 'Issue', run: 'Run', doc: 'Doc' }

const WorkRow = ({ work, now }: { readonly work: LinkedWork; readonly now: number }) => {
  const ref = githubRef(work)
  const link = <a href="#" onClick={e => e.preventDefault()} data-linked-work={work.kind} style={{ color: C.fg, textDecoration: 'none' }}>{work.title}</a>
  return <li style={{ display: 'flex', gap: 8, fontSize: 12, alignItems: 'baseline' }}>
    <span style={{ color: C.muted, minWidth: 34 }}>{workWord[work.kind]}</span>
    {ref === undefined ? link : <GithubHoverLink target={ref} now={now}>{link}</GithubHoverLink>}
  </li>
}

const excerptWord = { decision: 'Decision', failure: 'Failure', answer: 'Your answer', note: 'Note' } as const

/** The picked step: its card, linked work, conversation excerpts. */
const Inspector = ({ mission, linked, onOpen }: { readonly mission: ExploreMission; readonly linked: Linked; readonly onOpen: (ref: string) => void }) => {
  const step = mission.steps.find(s => s.id === linked.selected)
  if (step === undefined) return <aside style={{ padding: 16, color: C.muted, fontSize: 12 }}>Pick a step in the graph or the trace.</aside>
  return <aside aria-label={`${step.title} details`} style={{ display: 'flex', flexDirection: 'column', gap: 14, padding: 16 }}>
    <div style={{ fontSize: 12, lineHeight: '16px' }}><StepCard mission={mission} step={step} cursor={linked.cursor} /></div>
    {(step.work ?? []).length > 0 ? <section aria-label="Linked work"><div style={{ color: C.muted, fontSize: 11, marginBlockEnd: 4 }}>Linked work</div>
      <ul style={{ margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column', gap: 4 }}>{step.work!.map((w, i) => <WorkRow key={i} work={w} now={mission.now} />)}</ul></section> : null}
    {(step.excerpts ?? []).length > 0 ? <section aria-label="From the conversation"><div style={{ color: C.muted, fontSize: 11, marginBlockEnd: 4 }}>From the conversation</div>
      <ol style={{ margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column', gap: 8 }}>
        {step.excerpts!.map((e, i) => <li key={i} data-excerpt={e.kind} style={{ fontSize: 12, borderInlineStart: `2px solid ${e.kind === 'failure' ? C.bad : e.kind === 'answer' ? C.you : C.strong}`, paddingInlineStart: 8 }}>
          <div style={{ color: C.muted, fontSize: 11 }}>{excerptWord[e.kind]} · {e.who} · {clock(e.at)}</div>
          <div style={{ color: C.fg }}>{e.text}</div>
          <button type="button" onClick={() => onOpen(e.turn)} style={{ border: 0, padding: 0, background: 'transparent', color: C.soft, font: 'inherit', fontSize: 11, cursor: 'pointer', textDecoration: 'underline' }}>Open in {e.who === 'you' ? 'your conversation' : `${e.who}’s transcript`}</button>
        </li>)}
      </ol></section> : null}
  </aside>
}

/** Declaration as KDL with light highlighting; the TypeScript source it came from, once served. */
const Definition = ({ mission }: { readonly mission: ExploreMission }) => {
  const lines = (mission.kdl ?? '').split('\n')
  const tint = (line: string) => line.split(/("[^"]*"|\/\/.*$)/).map((part, i) => part.startsWith('//') ? <span key={i} style={{ color: C.muted }}>{part}</span> : part.startsWith('"') ? <span key={i} style={{ color: C.soft }}>{part}</span> : <span key={i}>{part}</span>)
  return <section aria-label="Definition" style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 10 }}>
    <div style={{ fontSize: 12, color: C.soft }}>Revision {mission.revisions.length} · published {clock(mission.revisions.at(-1)?.at ?? mission.startedAt)}{mission.source ? <> · from <code>{mission.source.repo}/{mission.source.path}</code> @ {mission.source.commit} <span style={{ color: C.muted }}>(TypeScript source view comes later)</span></> : null}</div>
    <pre aria-label="Mission KDL" tabIndex={0} style={{ margin: 0, padding: 12, border: `1px solid ${C.line}`, borderRadius: 8, overflow: 'auto', fontSize: 12, lineHeight: '18px', color: C.fg }}>
      {lines.map((line, i) => <div key={i} style={{ display: 'flex' }}><span aria-hidden="true" style={{ width: 28, color: C.muted, flexShrink: 0, textAlign: 'right', marginInlineEnd: 12 }}>{i + 1}</span><span>{tint(line)}</span></div>)}
    </pre>
  </section>
}

export const RefinedB = ({ mission, onOpen = () => {}, initialTab = 'execution', initialSelected = 'server', initialCursor }: { readonly mission: ExploreMission; readonly onOpen?: (ref: string) => void; readonly initialTab?: 'execution' | 'definition'; readonly initialSelected?: string; readonly initialCursor?: number }) => {
  const linked = useLinked(initialSelected, initialCursor)
  const [tab, setTab] = React.useState<'execution' | 'definition'>(initialTab)
  const [lower, setLower] = React.useState<'problems' | 'history'>('problems')
  const usage = totalUsage(mission)
  const hoveredStep = mission.steps.find(s => s.id === linked.hovered)
  return <article style={{ display: 'flex', flexDirection: 'column', minHeight: '100vh', background: C.canvas, color: C.fg, fontFamily: 'system-ui, sans-serif' }}>
    <div style={{ position: 'relative' }}>
      <Overview mission={mission} compact onSelect={linked.select} />
      <span data-mission-spend style={{ position: 'absolute', top: 14, right: 20, fontSize: 12, color: C.soft }}>{usd(usage.costUsd)} · {tokenCount(usage.tokens)} tokens</span>
    </div>
    <AgentsStrip mission={mission} linked={linked} onOpen={onOpen} />
    <Tabs current={tab} onChange={setTab} tabs={[{ id: 'execution', label: 'Execution' }, { id: 'definition', label: 'Definition' }]} />
    {tab === 'definition' ? <Definition mission={mission} /> : <>
      <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) 300px' }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10, padding: 16, minWidth: 0 }}>
          <Replay mission={mission} linked={linked} />
          <LinkedDag mission={mission} linked={linked} height={300} />
          <LinkedTrace mission={mission} linked={linked} />
        </div>
        <div style={{ borderInlineStart: `1px solid ${C.line}` }}><Inspector mission={mission} linked={linked} onOpen={onOpen} /></div>
      </div>
      <Tabs current={lower} onChange={setLower} tabs={[{ id: 'problems', label: 'Problems', count: mission.problems.length }, { id: 'history', label: 'History', count: mission.revisions.length }]} />
      <div style={{ padding: 16 }}>{lower === 'problems' ? <Problems mission={mission} onSelect={linked.select} /> : <History mission={mission} onSelect={linked.select} />}</div>
    </>}
    <StepCardHost anchor={linked.anchor} open={linked.cardOpen && hoveredStep !== undefined} onOpenChange={linked.keepCard}>
      {hoveredStep === undefined ? null : <StepCard mission={mission} step={hoveredStep} cursor={linked.cursor} />}
    </StepCardHost>
  </article>
}
