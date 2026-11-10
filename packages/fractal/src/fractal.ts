/**
 * The authoring surface, imported as `import { Fractal } from 'fractal'`.
 *
 * `agent`, `mission` and `schedule` declare the subject of an `agent.ts`, `mission.ts` or
 * `schedule.ts` module from flat fields plus `meta: import.meta`; the tree loader
 * (`fractal/tree`) derives each ID from the module's path. `defineAgent`, `defineMission` and
 * `defineSchedule` are the low-level forms whose `intent` may also be a context factory.
 */
export {
  agent,
  mission,
  schedule,
  defineAgent,
  defineMission,
  defineSchedule,
  pinMission,
  type AgentIntent,
  type MissionIntent,
  type ScheduleIntent,
  type PinnedMissionRef,
} from './define.ts'
export type { AgentRef, MissionRef, SubjectRef, SubjectDefinition } from './subject-ref.ts'
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
