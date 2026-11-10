import { Effect } from 'effect'

import { ProgramEntry, type HostContextV1, type ProgramExit } from './program-engine.ts'

export interface Recorded {
  readonly key: string
  readonly status: 'completed' | 'failed'
  readonly value?: unknown
  readonly error?: unknown
}

/** An in-memory host for one attempt; `record` keeps what a daemon would journal. */
export const fakeHost = (
  options: {
    readonly payload?: Readonly<Record<string, string>>
    readonly journal?: Iterable<readonly [string, ProgramExit]>
    readonly cwd?: string
    readonly child?: HostContextV1['child']
    /** Fails every journal write, as a daemon nack does. */
    readonly nack?: string
    readonly ask?: HostContextV1['ask']
    readonly spawn?: HostContextV1['spawn']
  } = {},
) => {
  const recorded: Recorded[] = []
  const started: string[] = []
  const controller = new AbortController()
  const context: HostContextV1 = {
    step: 'step-run/gen-1/program',
    attempt: 1,
    run: 'mission-run/run-1',
    payload: options.payload ?? { change: 'abc' },
    env: { ST_MISSION_RUN: 'mission-run/run-1' },
    cwd: options.cwd ?? '/work',
    journal: new Map(options.journal ?? []),
    signal: controller.signal,
    started: (key) => void started.push(key),
    record: async ({ key, status, value, error }) => {
      if (options.nack !== undefined) throw new Error(options.nack)
      // The daemon stores JSON: anything else would not replay.
      recorded.push(JSON.parse(JSON.stringify({ key, status, value, error })))
    },
    child: options.child ?? (() => Promise.reject(new Error('unexpected child run'))),
    ask: options.ask ?? (() => Promise.reject(new Error('unexpected ask'))),
    spawn:
      options.spawn ??
      (async (argv) => ({ code: 0, signal: null, stdout: argv.join(' '), stderr: '' })),
  }
  return { context, recorded, started, cancel: () => controller.abort() }
}

/** The journal a daemon replays from what an attempt recorded. */
export const journalOf = (recorded: readonly Recorded[]) =>
  recorded.map((entry): readonly [string, ProgramExit] => [
    entry.key,
    entry.status === 'completed'
      ? { _tag: 'Success', value: entry.value }
      : { _tag: 'Failure', error: entry.error },
  ])

/** Runs one attempt of `program` on `host`, as the program host does. */
export const start = (
  program: { readonly [ProgramEntry]: (host: HostContextV1) => Promise<ProgramExit> },
  host: { readonly context: HostContextV1 },
) => Effect.promise(() => program[ProgramEntry](host.context))
