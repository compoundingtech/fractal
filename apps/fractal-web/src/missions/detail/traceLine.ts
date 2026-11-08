import { runningMs, span, usd, waitedMs, type DetailStep } from './detailModel.ts'

// A trace row's second line: who and what it cost, the user's pick. `agent · ran 5h 30m · $11 · 3 tries`, then
// `· waited on you 5h` (drawn amber) only when the step waited on the person. Spend that no usage row names is left
// out rather than shown as $0.

export interface TraceLine {
  /** The neutral parts, joined with ` · `. */
  readonly parts: readonly string[]
  /** The wait on the person, when there was one. */
  readonly you: string | undefined
}

export const traceLine = (step: DetailStep, agentLabel: string | undefined, now: number): TraceLine => {
  const ran = runningMs(step, now)
  const waitedOnYou = waitedMs(step, now, 'you')
  return {
    parts: [
      agentLabel ?? step.owner.label,
      ...(ran > 0 ? [`ran ${span(ran)}`] : step.state === 'pending' || step.state === 'ready' ? ['not started'] : []),
      ...(step.spend === undefined ? [] : [usd(step.spend.costUsd)]),
      ...(step.attemptCount > 1 ? [`${step.attemptCount} tries`] : []),
    ],
    you: waitedOnYou > 0 ? `waited on you ${span(waitedOnYou)}` : undefined,
  }
}
