/** HTTP-only mutation ports; no collections socket is opened. */
import { ClientError, FractalClient, type ActionOf, type ErrorEnvelope } from '@fractal/client'
import { ActionResult, Snapshot, decodeUnknownSync } from '@fractal/client/schema'
import * as Data from 'effect/Data'
import * as Effect from 'effect/Effect'
import type { FractalOptions } from './read.ts'
import { traceContextOf } from './trace.ts'

/** The daemon refused an action; retain its wire response for caller-specific handling. */
export class ActionRefused extends Data.TaggedError('ActionRefused')<{
  readonly response: ErrorEnvelope
  readonly status: number
  readonly message: string
}> {}
/** No usable action response arrived. Delivery may still have happened. */
export class ActionTransportFailure extends Data.TaggedError('ActionTransportFailure')<{
  readonly cause: unknown
  readonly message: string
}> {}
/** Refusal or uncertain transport failure from an action invocation. */
export type ActionFailure = ActionRefused | ActionTransportFailure

/** Generated message.send input, allowing immutable draft tags. */
export type MessageSendInput = Omit<
  ActionOf<'message.send'>,
  'api_version' | 'type' | 'parameters'
> & {
  readonly parameters: Omit<ActionOf<'message.send'>['parameters'], 'tags'> & {
    readonly tags?: readonly string[]
  }
}

const decodeActionResult = decodeUnknownSync(ActionResult)
const decodeSnapshot = decodeUnknownSync(Snapshot)

const errorMessage = (error: unknown) => (error instanceof Error ? error.message : String(error))
const actionFailure = (cause: unknown): ActionFailure =>
  cause instanceof ClientError
    ? new ActionRefused({
        response: cause.response,
        status: cause.status,
        message: cause.response.message,
      })
    : new ActionTransportFailure({ cause, message: errorMessage(cause) })

/** HTTP-only arrangement actions; sharing this port never opens another collections socket. */
export interface ArrangementActionPort {
  readonly snapshot: Effect.Effect<string, ActionFailure>
  readonly submitAction: (request: ActionOf<'arrangement.edit'>) => Effect.Effect<typeof ActionResult.Type, ActionFailure>
}

export const arrangementActions = (client: Pick<FractalClient, 'capabilities' | 'arrangementEdit'>): ArrangementActionPort => ({
  snapshot: Effect.tryPromise({
    try: () => client.capabilities(),
    catch: actionFailure,
  }).pipe(
    Effect.flatMap((envelope) => Effect.try({
      try: () => decodeSnapshot(envelope.snapshot).id,
      catch: actionFailure,
    })),
    Effect.withSpan('st3.snapshot'),
  ),
  submitAction: Effect.fn('st3.arrangement.edit')((request: ActionOf<'arrangement.edit'>) =>
    Effect.tryPromise({
      try: () => client.arrangementEdit(request),
      catch: actionFailure,
    }).pipe(
      Effect.flatMap((envelope) => Effect.try({
        try: () => decodeActionResult(envelope.value),
        catch: actionFailure,
      })),
    )),
})

export interface MutationPort {
  readonly snapshot: Effect.Effect<string, ActionFailure>
  readonly messageSend: (request: MessageSendInput) => Effect.Effect<ActionResult, ActionFailure>
}

/** Fresh snapshots and message sending without a collections connection. */
export const makeMutations = (options: FractalOptions): MutationPort => {
  const client = new FractalClient({
    baseUrl: options.baseUrl,
    ...(options.credential === undefined ? {} : { credential: options.credential }),
    ...(options.client === undefined ? {} : { client: options.client }),
    fetchImpl: options.fetch ?? globalThis.fetch.bind(globalThis),
    ...(options.traceContext === undefined ? {} : { traceContext: options.traceContext }),
  })
  const sdkRequest = <A, E>(name: string, request: (traced: FractalClient) => Effect.Effect<A, E>) =>
    Effect.suspend(() => {
      const parent = options.parentSpan?.()
      return Effect.currentSpan.pipe(
        Effect.orDie,
        Effect.flatMap((span) => request(client.withTraceContext(() => traceContextOf(span)))),
        Effect.withSpan(name, parent === undefined ? {} : { parent }),
      )
    })
  return {
    snapshot: sdkRequest('st3.snapshot', (traced) =>
      Effect.tryPromise({ try: () => traced.capabilities(), catch: actionFailure }).pipe(
        Effect.flatMap((envelope) => Effect.try({
          try: () => decodeSnapshot(envelope.snapshot).id, catch: actionFailure,
        })),
      )),
    messageSend: (request: MessageSendInput) =>
      sdkRequest('st3.messageSend', (traced) =>
        Effect.tryPromise({
          try: () => {
            const { tags, ...parameters } = request.parameters
            return traced.messageSend({
              id: request.id,
              idempotency_key: request.idempotency_key,
              fence: request.fence,
              parameters: { ...parameters, ...(tags === undefined ? {} : { tags: [...tags] }) },
            })
          },
          catch: actionFailure,
        }).pipe(
          Effect.flatMap((envelope) => Effect.try({
            try: () => decodeActionResult(envelope.value), catch: actionFailure,
          })),
        )),
  }
}
