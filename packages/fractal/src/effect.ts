/**
 * Program-form missions, imported as `import { Fractal } from 'fractal/effect'`. The named exports
 * are the host contract a program bundle implements (docs/st3/program-engine.md §2).
 */
export * as Fractal from './fractal-effect.ts'
export {
  AskAnswer,
  AskCancelled,
  ChildFailed,
  ChildRun,
  type InputSchema,
  type ProgramIntent,
  type ProgramMission,
  type StepOptions,
} from './program.ts'
export {
  Program,
  ProgramEntry,
  type CommandOptions,
  type CommandResult,
  type HostContextV1,
  type ProgramExit,
  type ProgramServices,
} from './program-engine.ts'
