import type * as Native from '@fractal/client/schema'
import { Effect, SubscriptionRef } from 'effect'
import { makeOptimisticMutations, MutationRefused, type PendingMutation } from '../../data/optimisticMutation.ts'
import type { AlertAction, AlertActionPort, AlertActionResult, AlertAnswered } from './alert-action-port.ts'

type Answer = { readonly alert: AlertAnswered; readonly action: AlertAction }
export type OptimisticAttention = Native.Attention & {
  readonly optimisticAnswer?: { readonly phase: 'sending' | 'accepted' | 'failed'; readonly detail?: string; readonly action: AlertAction }
}
const project = (frame: readonly OptimisticAttention[], entry: PendingMutation<Answer>): readonly OptimisticAttention[] =>
  frame.map((alert) => alert.id !== entry.action.alert.id ? alert : {
    ...alert, optimisticAnswer: { phase: entry.phase, detail: entry.failure?.message, action: entry.action.action },
  })

/** One connection owns the answer overlays; every inbox and conversation sees the same state. */
export const optimisticAlertActions = (transport: AlertActionPort) => {
  const mutations = Effect.runSync(makeOptimisticMutations<readonly Native.Attention[], Answer>({
    initial: [], nextKey: () => `action/${crypto.randomUUID()}`,
    send: ({ key, action }) => Effect.tryPromise({
      try: () => transport.act({ ...action, key }),
      catch: () => new MutationRefused({ message: 'The answer could not be confirmed. Retry to confirm it.' }),
    }).pipe(Effect.flatMap((result) => result._tag === 'Sent' ? Effect.void : Effect.fail(new MutationRefused({ message: result.detail })))),
    project,
    reconcile: (frame, mutation) => {
      const row = frame.find((alert) => alert.id === mutation.action.alert.id)
      return row === undefined || row.state === 'resolved' || row.revision !== mutation.action.alert.revision ? 'applied' : 'pending'
    },
  }))
  const accept = Effect.fn('OptimisticAlertActions.accept')(function* (frame: readonly Native.Attention[]) {
    const before = yield* SubscriptionRef.get(mutations.state)
    yield* mutations.accept(frame)
    const after = yield* SubscriptionRef.get(mutations.state)
    for (const key of before.pending.keys()) if (!after.pending.has(key)) transport.forget?.(key)
  })
  const port: AlertActionPort = {
    act: (request) => Effect.runPromise(Effect.gen(function* () {
      const current = yield* SubscriptionRef.get(mutations.state)
      const previous = [...current.pending.values()].find((entry) => entry.action.alert.id === request.alert.id)
      const key = previous?.key ?? `action/${crypto.randomUUID()}`
      if (previous !== undefined) yield* mutations.retry(key)
      else yield* mutations.submit({ key, action: request, refusal: request.action.type === 'message.read' ||
        (request.action.type === 'work.done' && ['dismiss', 'read'].includes(request.action.answer?.id ?? '')) ? 'rollback' : 'retain' })
      const state = yield* SubscriptionRef.get(mutations.state)
      const failure = state.pending.get(key)?.failure ?? state.notices.find((notice) => notice.key === key)?.failure
      const result: AlertActionResult = failure === undefined ? { _tag: 'Sent' } : {
        _tag: 'Refused', detail: failure._tag === 'MutationRefused' ? failure.message : `Already answered by ${failure.by}.`,
      }
      if (!state.pending.has(key)) { transport.forget?.(key); yield* mutations.dismissNotice(key) }
      return result
    })),
  }
  return {
    port, state: mutations.state, accept,
    project: (frame: readonly Native.Attention[], pending: ReadonlyMap<string, PendingMutation<Answer>>) => {
      let projected: readonly OptimisticAttention[] = frame
      for (const entry of pending.values()) projected = project(projected, entry)
      return projected
    },
  }
}
