import { encodeKdl } from './index.ts'
import type { Agent, AgentReference, Declarations, Mission, Schedule, Step } from './index.ts'

const editor = {
  id: 'docs/editor', workspace: '.', launch: { command: 'true' },
} as const satisfies Agent
const writer = {
  id: 'docs/writer', workspace: '.', launch: { argv: ['tool', '--review'] },
  under: { agent: editor, reason: 'The editor reviews the work.' },
} as const satisfies Agent
const mission = {
  id: 'docs/daily-review', state: 'ready', reportTo: editor,
  goals: ['Review documentation.'], completion: 'all-steps-exhausted',
  steps: [{ id: 'review', assignedTo: writer, goals: ['Report corrections.'] }],
} as const satisfies Mission

export const declarations = {
  agents: [editor, writer], missions: [mission], schedules: [{
    id: 'docs/daily-review', host: 'local', every: '1d', anchor: '2026-01-01T09:00:00Z', catchUp: 'latest',
    work: { mission, revision: 'abc', workspace: '.' },
  }],
} as const satisfies Declarations
export const encoded: string = encodeKdl(declarations)
export const externalStep = { id: 'external', assignedTo: 'agent/docs/writer', goals: [] } as const satisfies Step
export const externalSchedule = {
  ...declarations.schedules[0], work: { mission: 'docs/daily-review', revision: 'abc', workspace: '.' },
} as const satisfies Schedule

export const catchUpAll = {
  ...externalSchedule, catchUp: 'all', maxCatchUp: 3,
} as const satisfies Schedule

export const maxGoalsMission = { ...mission, goals: ['one', 'two', 'three'], steps: [] } as const satisfies Mission
export const maxGoalsStep = { ...externalStep, goals: ['one', 'two', 'three'] } as const satisfies Step
export const dottedAgent = { ...editor, id: 'remote.editor' } as const satisfies Agent
export const dottedReference = 'agent/remote.editor' satisfies AgentReference

// Negative contracts fail compilation if an unsupported form becomes assignable.
// Type-level assertions avoid suppressing diagnostics on invalid runtime examples.
type Assert<T extends true> = T
type Rejects<TValue, TTarget> = TValue extends TTarget ? false : true
type Equal<TLeft, TRight> = [TLeft] extends [TRight] ? [TRight] extends [TLeft] ? true : false : false

export type AgentFields = Assert<Equal<keyof Agent, 'id' | 'workspace' | 'launch' | 'under'>>
export type MissionFields = Assert<Equal<keyof Mission, 'id' | 'state' | 'reportTo' | 'goals' | 'completion' | 'steps'>>
export type StepFields = Assert<Equal<keyof Step, 'id' | 'assignedTo' | 'goals'>>
export type ScheduleFields = Assert<Equal<keyof Schedule, 'id' | 'host' | 'every' | 'anchor' | 'catchUp' | 'maxCatchUp' | 'work'>>
export type WorkFields = Assert<Equal<keyof Schedule['work'], 'mission' | 'revision' | 'workspace'>>
export type DeclarationFields = Assert<Equal<keyof Declarations, 'agents' | 'missions' | 'schedules'>>
export type UnderFields = Assert<Equal<keyof NonNullable<Agent['under']>, 'agent' | 'reason'>>
export type NoEmptyArgv = Assert<Rejects<{ argv: readonly [] }, Agent['launch']>>
export type NoDualLaunch = Assert<Rejects<{ command: string; argv: readonly ['tool'] }, Agent['launch']>>
export type NoMissingLaunch = Assert<Rejects<{}, Agent['launch']>>
export type NoUndefinedArgv = Assert<Rejects<{ command: string; argv: undefined }, Agent['launch']>>
export type NoUndefinedCommand = Assert<Rejects<{ argv: readonly ['tool']; command: undefined }, Agent['launch']>>
export type NoBareAgentReference = Assert<Rejects<'docs/editor', AgentReference>>
export type NoMissionAgentReference = Assert<Rejects<'mission/docs/editor', AgentReference>>
export type NoHostFoldedAgentId = Assert<Rejects<'editor', Agent['id']>>
export type NoHostFoldedStringReference = Assert<Rejects<'agent/editor', AgentReference>>
export type NoHostFoldedObjectReference = Assert<Rejects<Omit<Agent, 'id'> & { id: 'editor' }, AgentReference>>
export type NoDynamicLaunch = Assert<Rejects<{ argv: readonly string[] }, Agent['launch']>>
export type NoActiveState = Assert<Rejects<'active', Mission['state']>>
export type NoPausedState = Assert<Rejects<'paused', Mission['state']>>
export type OnlyMissionStates = Assert<Equal<Mission['state'], 'draft' | 'ready' | 'retired'>>
export type OnlyCompletion = Assert<Equal<Mission['completion'], 'all-steps-exhausted'>>
export type NoGoalString = Assert<Rejects<string, Mission['goals']>>
export type NoStepGoalString = Assert<Rejects<string, Step['goals']>>
export type NoEmptyMissionGoals = Assert<Rejects<readonly [], Mission['goals']>>
export type NoFourMissionGoals = Assert<Rejects<readonly [string, string, string, string], Mission['goals']>>
export type NoFourStepGoals = Assert<Rejects<readonly [string, string, string, string], Step['goals']>>
export type NoUnboundedMissionGoals = Assert<Rejects<readonly string[], Mission['goals']>>
export type NoUnboundedStepGoals = Assert<Rejects<readonly string[], Step['goals']>>
export type NoEmptyDeclarations = Assert<Rejects<{}, Declarations>>
export type NoEmptyDeclarationCollections = Assert<Rejects<{ agents: readonly []; missions: readonly []; schedules: readonly [] }, Declarations>>
export type NoStepDependencies = Assert<Rejects<'dependsOn', keyof Step>>
export type NoCron = Assert<Rejects<'cron', keyof Schedule>>
export type OnlyCatchUpPolicies = Assert<Equal<Schedule['catchUp'], 'all' | 'latest' | 'skip'>>
type ScheduleBase = Omit<Schedule, 'catchUp' | 'maxCatchUp'>
export type NoUnboundedCatchUpAll = Assert<Rejects<ScheduleBase & { catchUp: 'all' }, Schedule>>
export type NoUndefinedCatchUpAllBound = Assert<Rejects<ScheduleBase & { catchUp: 'all'; maxCatchUp: undefined }, Schedule>>
export type NoLatestCatchUpBound = Assert<Rejects<ScheduleBase & { catchUp: 'latest'; maxCatchUp: number }, Schedule>>
export type NoSkipCatchUpBound = Assert<Rejects<ScheduleBase & { catchUp: 'skip'; maxCatchUp: number }, Schedule>>
export type NoImplicitRevision = Assert<Rejects<{ mission: string; workspace: string }, Schedule['work']>>
export type NoMissingUnderReason = Assert<Rejects<{ agent: AgentReference }, NonNullable<Agent['under']>>>
export type NoUndefinedCollections = Assert<Rejects<{ agents: undefined }, Declarations>>
