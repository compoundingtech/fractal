import { markdownSource, type MarkdownInline } from '@fractal/client/markdown'
import type * as Fractal from '@fractal/client/schema'
import { DateTime, Option } from 'effect'

import { markdownPlainText } from '../../../../packages/fractal-ui/src/assistant-ui/composition/markdown-source.ts'
import { missionSteps, type MissionPresentation, type PresentedStep } from './presentation.ts'

// The mission's plan as a dependency-ordered stepper: levels from the visualization's dependency edges (else the steps'
// blockers), steps sharing a level run side by side, and a level's depth is its indentation. The presentation's own
// step order is by state, which would scramble the plan, so it only supplies each step's word and owner here.

/** An optional decoded field: an Option, a DateTime, a number or nothing, as Unix ms. */
export const millis = (value: unknown): number | undefined => {
  if (value === null || value === undefined) return undefined
  if (Option.isOption(value)) return Option.isSome(value) ? millis(value.value) : undefined
  if (typeof value === 'number') return Number.isFinite(value) ? value : undefined
  if (DateTime.isDateTime(value)) return DateTime.toEpochMillis(value)
  return undefined
}
const text = (value: unknown): string | undefined => {
  if (value === null || value === undefined) return undefined
  if (Option.isOption(value)) return Option.isSome(value) ? text(value.value) : undefined
  return typeof value === 'string' && value !== '' ? value : undefined
}

/** An optional decoded Markdown field, kept Markdown; empty counts as absent. */
const markdown = (value: Option.Option<MarkdownInline> | MarkdownInline | null | undefined): MarkdownInline | undefined => {
  const field = Option.isOption(value) ? Option.getOrUndefined(value) : value ?? undefined
  return field === undefined || markdownSource(field) === '' ? undefined : field
}

export type PlanStep = PresentedStep & {
  readonly level: number
  /** Paths this step waits for. */
  readonly after: readonly string[]
  /** What the step is for, as its author wrote it (inline Markdown). */
  readonly goal: MarkdownInline | undefined
  readonly startedAt: number | undefined
  readonly completedAt: number | undefined
  readonly since: number | undefined
  /** The agent's last progress line (inline Markdown). */
  readonly progress: MarkdownInline | undefined
  readonly blockedReason: string | undefined
}

export type MissionPlan = {
  readonly levels: ReadonlyArray<readonly PlanStep[]>
  readonly steps: readonly PlanStep[]
  readonly edges: ReadonlyArray<{ readonly from: string; readonly to: string }>
}

export const missionPlan = (mission: Fractal.Mission, presentation: MissionPresentation): MissionPlan => {
  const raw = missionSteps(mission)
  const presented = new Map(presentation.steps.map(step => [step.id, step]))
  const visualization = Option.isOption(mission.visualization) ? Option.getOrUndefined(mission.visualization as Option.Option<Fractal.Visualization>) : (mission.visualization ?? undefined) as Fractal.Visualization | undefined
  const nodes = visualization?.nodes ?? []
  const nodePath = new Map(nodes.map(node => [node.id, node.path]))
  const byId = new Map(raw.map(step => [step.id as string, step.path]))
  const edges: Array<{ from: string; to: string }> = []
  for (const edge of visualization?.edges ?? []) {
    const fromId = text(edge.from as unknown)
    const from = fromId === undefined ? undefined : nodePath.get(fromId) ?? fromId
    const to = nodePath.get(edge.to) ?? edge.to
    if (from !== undefined && edge.kind === 'dependency') edges.push({ from, to })
  }
  if (edges.length === 0) {
    for (const step of raw) for (const blocker of step.blockers ?? []) {
      const from = byId.get(blocker)
      if (from !== undefined) edges.push({ from, to: step.path })
    }
  }
  const paths = new Set(raw.map(step => step.path))
  const known = edges.filter(edge => paths.has(edge.from) && paths.has(edge.to))
  const after = (path: string) => known.filter(edge => edge.to === path).map(edge => edge.from)
  const levelOf = new Map<string, number>()
  const level = (path: string, seen: ReadonlySet<string> = new Set()): number => {
    const cached = levelOf.get(path)
    if (cached !== undefined) return cached
    if (seen.has(path)) return 0
    const deps = after(path)
    const value = deps.length === 0 ? 0 : 1 + Math.max(...deps.map(dep => level(dep, new Set([...seen, path]))))
    levelOf.set(path, value)
    return value
  }
  // Declaration order where the graph names it, else run order.
  const order = new Map([...nodes.map(node => node.path), ...raw.map(step => step.path)].map((path, index) => [path, index] as const).reverse())
  const steps = raw.map((step): PlanStep => {
    const shown = presented.get(step.id)
    const goal = markdown(step.goals?.[0] ?? nodes.find(node => node.path === step.path)?.goals[0])
    return {
      id: step.id,
      path: step.path,
      // A title is a label: a goal standing in for one shows its words, without Markdown markers.
      title: shown?.title ?? text(step.title) ?? (goal === undefined ? undefined : markdownPlainText(markdownSource(goal))) ?? step.path,
      word: shown?.word ?? 'pending',
      owner: shown?.owner ?? { kind: 'nobody', label: 'nobody yet' },
      note: shown?.note ?? null,
      attempts: step.attempt,
      blockers: step.blockers ?? [],
      keep_open: step.keep_open === true,
      level: level(step.path),
      after: after(step.path),
      goal,
      startedAt: millis(step.started_at),
      completedAt: millis(step.completed_at),
      since: millis(step.since),
      progress: markdown(step.last_progress),
      blockedReason: text(step.blocked_reason),
    }
  }).sort((a, b) => a.level - b.level || (order.get(a.path) ?? 0) - (order.get(b.path) ?? 0))
  const depth = steps.reduce((max, step) => Math.max(max, step.level), -1) + 1
  const levels = Array.from({ length: depth }, (_, index) => steps.filter(step => step.level === index))
  return { levels, steps, edges: known }
}

/** How long a step took or has been going: done → started..completed; working → started..now. */
export const stepDuration = (step: PlanStep, now: number): number | undefined => {
  const start = step.startedAt ?? (step.word === 'working' ? step.since : undefined)
  if (start === undefined) return undefined
  const end = step.word === 'done' ? step.completedAt : step.word === 'working' ? now : undefined
  return end === undefined || end < start ? undefined : end - start
}

export type ActivityEntry = {
  readonly at: number
  readonly kind: 'completed' | 'claimed' | 'retry' | 'failed' | 'waiting' | 'run' | 'gate'
  readonly text: string
  readonly step?: string
  readonly agent?: string
}

/**
 * What happened, newest first, from what the mission serves: step starts (claims), completions, retries and failures,
 * run starts and outcomes, and gates opened on the person. Nothing is invented: a fact without a time is left out.
 */
export const missionActivity = ({ mission, plan, attention }: { readonly mission: Fractal.Mission; readonly plan: MissionPlan; readonly attention: readonly Fractal.Attention[] }): readonly ActivityEntry[] => {
  const entries: ActivityEntry[] = []
  for (const step of plan.steps) {
    const who = step.owner.kind === 'agent' ? step.owner.label : step.owner.kind === 'st' ? 'Fractal' : undefined
    const agent = step.owner.kind === 'agent' ? step.owner.agent : undefined
    const base = { step: step.id, ...(agent === undefined ? {} : { agent }) }
    if (step.startedAt !== undefined) entries.push({ ...base, at: step.startedAt, kind: 'claimed', text: who === undefined ? `Started ${step.title}` : `${who} took ${step.title}` })
    if (step.completedAt !== undefined && step.word === 'done') entries.push({ ...base, at: step.completedAt, kind: 'completed', text: `${step.title} done` })
    if (step.word === 'failed' && step.since !== undefined) entries.push({ ...base, at: step.since, kind: 'failed', text: `${step.title} failed${step.attempts > 1 ? ` on attempt ${step.attempts}` : ''}` })
    else if (step.attempts > 1 && step.since !== undefined) entries.push({ ...base, at: step.since, kind: 'retry', text: `${step.title}: attempt ${step.attempts}` })
    if ((step.word === 'waiting' || step.word === 'ready') && step.since !== undefined && (step.blockedReason ?? step.note) !== undefined && (step.blockedReason ?? step.note) !== null) entries.push({ ...base, at: step.since, kind: 'waiting', text: `${step.title}: ${step.blockedReason ?? step.note}` })
  }
  for (const run of mission.run_details ?? []) {
    const started = millis(run.started_at)
    if (started !== undefined) entries.push({ at: started, kind: 'run', text: `Run ${run.id.split('/').pop()} started` })
    const outcome = Option.isOption(run.outcome) ? Option.getOrUndefined(run.outcome) : run.outcome ?? undefined
    if (outcome !== undefined && outcome !== null) entries.push({ at: millis(outcome.at) ?? 0, kind: outcome.status === 'failed' ? 'failed' : 'run', text: `Run ${run.id.split('/').pop()} ${outcome.status}${outcome.reason === '' ? '' : `: ${outcome.reason}`}` })
  }
  for (const item of attention) {
    const at = millis(item.requested_at)
    if (at !== undefined) entries.push({ at, kind: 'gate', text: `Asked you: ${item.title}` })
  }
  return entries.filter(entry => entry.at > 0).sort((a, b) => b.at - a.at)
}
