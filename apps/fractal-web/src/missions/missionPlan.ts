import { markdownSource, type MarkdownInline } from '@fractal/client/markdown'
import type * as Fractal from '@fractal/client/schema'
import { DateTime, Option } from 'effect'

import { markdownPlainText } from '../../../../packages/fractal-ui/src/assistant-ui/composition/markdown-source.ts'
import { missionSteps, type MissionPresentation, type PresentedStep } from './presentation.ts'

// The mission's plan in dependency order: levels from the visualization's dependency edges (else the steps' blockers),
// then declaration order within a level. The presentation's own step order is by state, which would scramble the
// plan, so it only supplies each step's word and owner here.

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
      since: millis(step.since),
      progress: markdown(step.last_progress),
      blockedReason: text(step.blocked_reason),
    }
  }).sort((a, b) => a.level - b.level || (order.get(a.path) ?? 0) - (order.get(b.path) ?? 0))
  const depth = steps.reduce((max, step) => Math.max(max, step.level), -1) + 1
  const levels = Array.from({ length: depth }, (_, index) => steps.filter(step => step.level === index))
  return { levels, steps, edges: known }
}
