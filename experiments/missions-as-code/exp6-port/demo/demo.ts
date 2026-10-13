// Demo scripts as Effect programs (experiment 6 addendum).
//
// A demo's beats become ops of kind "beat" on the mission SDK's engine. Its durable clock does
// the timing, so a whole demo can be played out in virtual time at once. compile.ts uses that
// to write the beats block of the plain demo file (`st demo play`) that the program means.
//
// The seed stays in the demo's KDL: it is declarations, which is what KDL is for.

import { Effect } from "effect"
import { opEffect } from "../../sdk/engine.ts"
import type { Json } from "../../sdk/history.ts"

export type Attrs = Record<string, string | boolean | undefined>

export interface Beat {
  readonly action: string
  readonly target?: string // the first positional: an agent, a run, a document
  readonly positional?: ReadonlyArray<string> // further positionals, as "Bash" for tool
  readonly attrs?: Attrs
  readonly text?: string // the trailing text
  readonly answers?: ReadonlyArray<{ id: string; label: string; outcome?: string; then?: string }>
}

const beat = (b: Beat): Effect.Effect<Json> => Effect.orDie(opEffect("beat", b.action, b as unknown as Json))

const ms = (d: string): number => {
  const m = /^([\d.]+)(ms|s|m|h)$/.exec(d)
  if (!m) throw new Error(`bad duration ${d}`)
  return Number(m[1]) * { ms: 1, s: 1000, m: 60_000, h: 3_600_000 }[m[2] as "ms"]
}

// The beat fires `d` after the previous one; without it, a beat waits for the advance key.
export const after =
  (d: string) =>
  <A, E, R>(self: Effect.Effect<A, E, R>): Effect.Effect<A, E, R> =>
    Effect.andThen(Effect.sleep(ms(d)), self)
export const key = Effect.asVoid(beat({ action: "key" }))
const onKey = <A, E, R>(self: Effect.Effect<A, E, R>) => Effect.andThen(key, self)

// Actions, one per row of the format's table. Each is the st command a real seat would run.
export const open = (target: string, attrs: Attrs = {}) => Effect.asVoid(beat({ action: "open", target, attrs }))
export const focus = (target: string, attrs: Attrs = {}) => Effect.asVoid(beat({ action: "focus", target, attrs }))
export const say = (agent: string, text: string, attrs: Attrs = {}) => Effect.asVoid(beat({ action: "say", target: agent, attrs, text }))
export const tool = (agent: string, name: string, attrs: Attrs) => Effect.asVoid(beat({ action: "tool", target: agent, positional: [name], attrs }))
export const send = (from: string, to: string, text: string, attrs: Attrs = {}) =>
  Effect.asVoid(beat({ action: "send", attrs: { from, to, ...attrs }, text }))
const stepAction = (action: string) => (agent: string, run: string, step: string, text?: string) =>
  Effect.asVoid(beat({ action, target: agent, attrs: { run, step }, ...(text ? { text } : {}) }))
export const claim = stepAction("claim")
export const progress = stepAction("progress")
export const complete = stepAction("complete")
export const fail = stepAction("fail")
export const type = (text: string, attrs: Attrs = {}) => Effect.asVoid(beat({ action: "type", target: text, attrs }))
export const pause = Effect.asVoid(beat({ action: "pause" }))

// An alert. The answer comes back to the program, which a beats file cannot do.
export const ask = (agent: string, question: string, attrs: Attrs, answers: NonNullable<Beat["answers"]>) =>
  Effect.map(beat({ action: "ask", target: agent, attrs, text: question, answers }), (a) => (a as { id: string }).id)

export { onKey }
