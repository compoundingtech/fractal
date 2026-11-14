import { Effect, Option, Schema, type Scope } from 'effect'

import { ChildFailed, ChildRun } from './program.ts'
import { CommandResultSchema, keyed, Program, ResourceCommandScope, ResourceError, type CommandOptions, type CommandResult, type ResourceRequest } from './program-engine.ts'
import { normalizeDuration, type DurationInput } from './values.ts'

export const LaneMembership = Schema.Struct({
  membershipId: Schema.String,
  authorityEpoch: Schema.String,
  generation: Schema.String,
  coverage: Schema.Literals(['logical', 'enforced']),
})
export type LaneMembership = typeof LaneMembership.Type

export interface LaneOptions {
  readonly host: string
  readonly maxMembers?: number
  /** Bytes, charged by the host accountant; not a client-side semaphore. */
  readonly capacity?: { readonly cores: number; readonly memory: number }
  readonly coverage?: 'logical' | 'enforced'
  readonly admissionTimeout?: DurationInput
}

export interface LaneCommandOptions extends CommandOptions {
  /** Payload budget; admission and retirement have independent clocks. */
  readonly timeout?: DurationInput
}

export interface Lane {
  readonly membership: LaneMembership
  exec(id: string, argv: readonly string[], options?: LaneCommandOptions): Effect.Effect<CommandResult, ResourceError, Program>
  run(id: string, mission: string, input: Readonly<Record<string, string>>): Effect.Effect<ChildRun, ResourceError | ChildFailed, Program>
}

const invoke = <TValue>(
  program: Program['Service'],
  request: ResourceRequest,
  success: Schema.Codec<TValue, unknown>,
): Effect.Effect<TValue, ResourceError> =>
  Effect.flatMap(Effect.promise((signal) => program.host.resource(request, signal)), (exit) =>
    exit._tag === 'Success'
      ? Effect.orDie(Schema.decodeUnknownEffect(success)(exit.value))
      : Effect.flatMap(Effect.orDie(Schema.decodeUnknownEffect(ResourceError)(exit.error)), Effect.fail),
  ).pipe(Effect.withSpan(request.operation))

/**
 * Acquire a host-local lane in the caller's scope. Every evaluation reattaches/revalidates the
 * exact request at the native boundary, including replay. Closing the scope awaits retirement;
 * a process exit or a journaled success does not itself free host capacity.
 * Within a durable activity, request identities belong to that activity; the physical lane name
 * remains shared policy. Repeated acquisitions in the same scope use successive occurrence keys.
 */
export const lane = (
  name: string,
  options: LaneOptions,
): Effect.Effect<Lane, ResourceError, Program | Scope.Scope> =>
  Effect.flatMap(Effect.serviceOption(ResourceCommandScope), (scope) =>
    keyed(Option.match(scope, {
      onNone: () => `resource.lane/${name}`,
      onSome: (activity) => `${activity}/resource.lane/${name}`,
    }), (key, program) =>
    Effect.map(Effect.acquireRelease(
      invoke(program, {
        key,
        operation: 'resource.lane.acquire',
        payload: {
          name,
          ...options,
          coverage: options.coverage ?? 'logical',
          ...(options.admissionTimeout === undefined ? {} : {
            admissionTimeout: normalizeDuration({ input: options.admissionTimeout }),
          }),
        },
      }, LaneMembership).pipe(Effect.interruptible),
      (membership) => invoke(program, {
        key: `${key}/release`, operation: 'resource.lane.release', payload: { membership },
      }, Schema.NullOr(Schema.Void)).pipe(Effect.orDie),
    ), (membership): Lane => ({
      membership,
      exec: (id, argv, commandOptions) => keyed(`${key}/exec/${id}`, (executionKey) =>
        invoke(program, {
          key: executionKey,
          operation: 'resource.lane.exec',
          payload: {
            membership,
            argv,
            options: {
              ...commandOptions,
              ...(commandOptions?.timeout === undefined ? {} : {
                timeout: normalizeDuration({ input: commandOptions.timeout }),
              }),
            },
          },
        }, CommandResultSchema),
      ),
      run: (id, mission, input) => keyed(`${key}/run/${id}`, (executionKey) =>
        Effect.flatMap(Effect.promise((signal) => program.host.resource({
          key: executionKey, operation: 'resource.lane.run', payload: { membership, mission, input },
        }, signal)), (exit) => exit._tag === 'Success'
          ? Effect.orDie(Schema.decodeUnknownEffect(ChildRun)(exit.value))
          : Effect.flatMap(Effect.orDie(Schema.decodeUnknownEffect(Schema.Union([ResourceError, ChildFailed]))(exit.error)), Effect.fail)),
      ),
    })),
    ),
  ).pipe(Effect.withSpan('Fractal.resources.lane'))

/** Host-native resource capabilities; Effect owns scope and control flow. */
export const resources = { lane }
