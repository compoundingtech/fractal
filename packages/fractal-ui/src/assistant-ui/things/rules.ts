/**
 * The thing kit's pure rules, decided once for every kind: ages, act colour, accessible names and section order.
 * No React here, so the rules run under plain Node in `rules.unit.test.ts`.
 */
import { formatRelative, nextRelativeChange } from '../../time/human-time.ts'
import type { Observed, ThingFace } from './face.ts'

type KnownFace = Extract<Observed<ThingFace>, { readonly _tag: 'Known' }>

/** `formatRelative` without its verb: `4m`, `3h`, `just now`, `yesterday 14:02`. The card carries the verb. */
export const bareAge = (at: number, now: number): string => formatRelative(at, now).replace(/ ago$/, '')

/** A row's age: the kind's own moment, bare; a stale observation says when it was last seen instead. */
export const rowAge = (observed: KnownFace, now: number): string | undefined => {
  if (observed.freshness === 'stale') return `seen ${bareAge(observed.observedAt, now)}`
  return observed.value.at === undefined ? undefined : bareAge(observed.value.at.at, now)
}

/**
 * When a row's age text next changes, for the host's clock deadline: the kit is controlled by `now` and schedules
 * nothing itself. A host takes the earliest over its rows.
 */
export const nextAgeChange = (observed: Observed<ThingFace>, now: number): number | undefined => {
  if (observed._tag !== 'Known') return undefined
  if (observed.freshness === 'stale') return nextRelativeChange(observed.observedAt, now)
  return observed.value.at === undefined ? undefined : nextRelativeChange(observed.value.at.at, now)
}

/** A card's age keeps the kind's verb: `updated 4m`, `started 2h`. */
export const cardAge = (face: ThingFace, now: number): string | undefined =>
  face.at === undefined ? undefined : `${face.at.verb} ${bareAge(face.at.at, now)}`

/**
 * The act label's paint. Amber means you must act, so a read-only thing's ask is drawn without colour; a failure
 * stays red. Staleness never greys a label: the age already says `seen …`.
 */
export type ActPaint = 'act' | 'failed' | 'quiet'
export const actPaint = (observed: KnownFace): ActPaint | undefined => {
  const act = observed.value.act
  if (act === undefined) return undefined
  return act.tone === 'act' && observed.readOnly === true ? 'quiet' : act.tone
}

/** The accessible name of an openable row or chip: everything it paints, whatever the width drops. */
export const thingLabel = (observed: KnownFace, now: number): string => {
  const face = observed.value
  const identity = [face.kind, face.id, face.title].filter(part => part !== undefined && part !== '').join(' ')
  const state = face.act?.label ?? face.mark.word
  return [identity, state, rowAge(observed, now)].filter(part => part !== undefined && part !== '').join(', ')
}

/** 0: needs you or failed · 1: active · 2: everything else, including what is not known yet. */
export const thingRank = (observed: Observed<ThingFace>): 0 | 1 | 2 => {
  if (observed._tag !== 'Known') return 2
  const paint = actPaint(observed)
  if (paint === 'act' || paint === 'failed') return 0
  return observed.value.mark.shape === 'filled' ? 1 : 2
}

const momentOf = (observed: Observed<ThingFace>): number =>
  observed._tag === 'Known' ? observed.value.at?.at ?? Number.NEGATIVE_INFINITY : Number.NEGATIVE_INFINITY

/**
 * Section order as a comparator: needs-you and failed first, then active, then newest. Undated things tie, so a
 * stable sort keeps their given order after dated ones of the same rank.
 */
export const compareThings = (a: Observed<ThingFace>, b: Observed<ThingFace>): number => {
  const byRank = thingRank(a) - thingRank(b)
  if (byRank !== 0) return byRank
  const left = momentOf(a), right = momentOf(b)
  return left === right ? 0 : right > left ? 1 : -1
}

/** `compareThings` over section items, without mutating them. */
export const sortThings = <T extends { readonly observed: Observed<ThingFace> }>(items: readonly T[]): readonly T[] =>
  [...items].sort((a, b) => compareThings(a.observed, b.observed))
