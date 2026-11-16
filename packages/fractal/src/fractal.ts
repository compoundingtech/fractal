/**
 * The authoring surface, imported as `import { Fractal } from 'fractal'`.
 *
 * `agent`, `mission` and `schedule` declare the subject of an `agent.ts`, `mission.ts` or
 * `schedule.ts` module from flat fields plus `meta: import.meta`; the tree loader
 * (`fractal/tree`) derives each ID from the module's path. `objective` declares the objective of
 * an `objective.ts` module, measured by an `indicator` and answered by a `response`.
 * `defineAgent`, `defineMission`, `defineSchedule` and `defineObjective` are the low-level forms
 * whose `intent` may also be a context factory.
 */
export {
  agent,
  mission,
  schedule,
  defineAgent,
  defineMission,
  defineSchedule,
  defineObjective,
  pinMission,
  type AgentIntent,
  type MissionIntent,
  type ScheduleIntent,
  type ObjectiveIntent,
  type PinnedMissionRef,
} from './define.ts'
export type { AgentRef, MissionRef, SubjectRef, SubjectDefinition } from './subject-ref.ts'
export {
  objective,
  indicator,
  response,
  type AgentTarget,
  type ClaimEvent,
  type Duration,
  type Fault,
  type Indicator,
  type Interval,
  type Objective,
  type ObjectiveInput,
  type Target,
  type Window,
} from './objective.ts'
export {
  step,
  gate,
  person,
  completed,
  failed,
  terminal,
  omp,
  Fragment,
  Mission,
  Round,
  missionWire,
  type StepHandle,
  type StepInput,
  type MissionInput,
  type PersonReference,
} from './mod.ts'
export { childMission, doc, input, pr, product, runId, t } from './run-values.ts'
export { Launch, Review, Timing } from './values.ts'
