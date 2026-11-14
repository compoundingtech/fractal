import type { AgentSchema, MissionInput, ScheduleSchema, WorkSchema } from './mod.ts'
import type { ObjectiveInput } from './objective.ts'
import type {
  SubjectKind,
  SubjectDefinition,
  AgentRef,
  MissionRef,
} from './subject-ref.ts'

/** Exact source revision binding; zero requests the graph renderer's ST_REVISION receipt. */
export interface PinnedMissionRef {
  readonly _tag: 'PinnedMissionRef'
  readonly subject: MissionRef
  readonly revision: string
}

/**
 * Pins imported mission work, never its moving published head. Without an explicit hash,
 * the graph renderer binds the zero sentinel to this source mission's native revision.
 * A step's `waitFor` requires an explicit revision and is exclusive with native `usesMission`;
 * scoped producer handles authored with `step({ waitFor })` retain their native semantics.
 */
export const pinMission = ({
  subject,
  revision = '0'.repeat(64),
}: {
  readonly subject: MissionRef
  readonly revision?: string
}): PinnedMissionRef => {
  if (/^[0-9a-fA-F]{64}$/u.test(revision) === false)
    throw new TypeError('Pinned mission revision must be a 64-hex native revision')
  return Object.freeze({ _tag: 'PinnedMissionRef', subject, revision })
}

type WorkWire = typeof WorkSchema.Encoded

/** Agent fields without `id`; supervisor targets may be typed seat references. */
export type AgentIntent = Omit<typeof AgentSchema.Encoded, 'id' | 'under'> & {
  readonly under?: ReadonlyArray<{
    readonly target: AgentRef | NonNullable<(typeof AgentSchema.Encoded)['under']>[number]['target']
    readonly reason?: string
  }>
}

type TreeStep<T> = T extends { readonly round: infer R }
  ? Omit<T, 'round'> & { readonly round: TreeRound<R> }
  : {
      [K in keyof T]: K extends 'assignedTo' ? T[K] | AgentRef : T[K]
    } & (
      | { readonly waitFor?: never }
      | { readonly waitFor: PinnedMissionRef; readonly usesMission?: never }
    )

type TreeSteps<T> = T extends readonly unknown[] ? { [K in keyof T]: TreeStep<T[K]> } : T

type TreeRound<T> = {
  [K in keyof T]: K extends 'steps' | 'finally' ? TreeSteps<T[K]> : T[K]
}

/** Path-derived mission ID with current authoring handles and typed seat references. */
export type MissionIntent = Omit<
  MissionInput<string>,
  'id' | 'steps' | 'finally' | 'schedule' | 'reportTo' | 'assignedTo'
> & {
  readonly reportTo: AgentRef | MissionInput<string>['reportTo']
  readonly assignedTo?: AgentRef | MissionInput<string>['assignedTo']
  readonly steps: ReadonlyArray<TreeStep<MissionInput<string>['steps'][number]>>
  readonly finally?: ReadonlyArray<TreeStep<NonNullable<MissionInput<string>['finally']>[number]>>
}

type TreeSchedule<TSchedule> = TSchedule extends unknown
  ? Omit<TSchedule, 'work'> & {
      readonly work: Omit<WorkWire, 'mission'> & {
        readonly mission: MissionRef | PinnedMissionRef | WorkWire['mission']
      }
    }
  : never

/** Mission fields for a schedule file; its work may name a typed mission reference. */
export type ScheduleIntent = MissionIntent & {
  readonly schedule: TreeSchedule<typeof ScheduleSchema.Encoded>
}

/** `meta` is the defining module's `import.meta`; `intent` is a value or a context factory. */
type DefineArgs<I, C> = {
  readonly meta: { readonly url: string }
  readonly intent: I | ((context: C) => I)
}

type Define<K extends SubjectKind, I> = <C = never>(
  args: DefineArgs<I, C>,
) => SubjectDefinition<K, I, C>

const define =
  <K extends SubjectKind>(kind: K) =>
  <I, C>({ meta, intent }: DefineArgs<I, C>): SubjectDefinition<K, I, C> => ({
    _tag: 'SubjectRef',
    kind,
    url: meta.url,
    intent,
  })

/** Defines the seat declared by an `agent.ts` module. */
export const defineAgent: Define<'agent', AgentIntent> = define('agent')

/** Defines the mission declared by a `mission.ts` module. */
export const defineMission: Define<'mission', MissionIntent> = define('mission')

/** Defines the scheduled mission declared by a `schedule.ts` module. */
export const defineSchedule: Define<'schedule', ScheduleIntent> = define('schedule')

/** Objective fields for an `objective.ts` module; the name, if given, must be its path ID. */
export type ObjectiveIntent = ObjectiveInput

/** Defines the objective declared by an `objective.ts` module; see `Fractal.objective`. */
export const defineObjective: Define<'objective', ObjectiveIntent> = define('objective')

/** The defining module's `import.meta`; the loader derives the subject ID from the module's path. */
type Meta = { readonly meta: { readonly url: string } }

/**
 * Declares the agent of an `agent.ts` module from flat fields:
 * `export default agent({ meta: import.meta, workspace: '/work/garden', ... })`.
 * The same declaration as `defineAgent({ meta, intent })`, without the context factory form.
 */
export const agent = ({
  meta,
  ...intent
}: Meta & AgentIntent): SubjectDefinition<'agent', AgentIntent, never> =>
  defineAgent({ meta, intent })

/** Declares the mission of a `mission.ts` module from flat fields; see {@link agent}. */
export const mission = ({
  meta,
  ...intent
}: Meta & MissionIntent): SubjectDefinition<'mission', MissionIntent, never> =>
  defineMission({ meta, intent })

/** Declares the scheduled mission of a `schedule.ts` module from flat fields; see {@link agent}. */
export const schedule = ({
  meta,
  ...intent
}: Meta & ScheduleIntent): SubjectDefinition<'schedule', ScheduleIntent, never> =>
  defineSchedule({ meta, intent })
