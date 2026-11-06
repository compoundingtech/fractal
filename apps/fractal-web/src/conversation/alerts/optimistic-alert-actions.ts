import type * as Native from '@fractal/client/schema'
import { Effect, SubscriptionRef } from 'effect'
import { makeOptimisticMutations, MutationRefused, type MutationNotice, type PendingMutation } from '../../data/optimisticMutation.ts'
import type { AlertAction, AlertActionPort, AlertActionResult, AlertAnswered } from './alert-action-port.ts'

type Answer = { readonly alert: AlertAnswered; readonly action: AlertAction }
export type OptimisticAttention = Native.Attention & {
  readonly optimisticAnswer?: { readonly phase: 'sending' | 'accepted' | 'failed'; readonly detail?: string; readonly action: AlertAction }
  readonly optimisticNote?: string
}
const project = (frame: readonly OptimisticAttention[], entry: PendingMutation<Answer>): readonly OptimisticAttention[] =>
  entry.refusal === 'rollback' && entry.phase !== 'failed' ? frame.filter((alert) => alert.id !== entry.action.alert.id)
    : frame.map((alert) => alert.id !== entry.action.alert.id ? alert : {
      ...alert, optimisticAnswer: { phase: entry.phase, detail: entry.failure?.message, action: entry.action.action },
    })

/** One connection owns the answer overlays; every inbox and conversation sees the same state. */
export const optimisticAlertActions = (transport: AlertActionPort) => {
  const noticeTargets = new Map<string, AlertAnswered>()
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
    for (const key of before.pending.keys()) if (!after.pending.has(key)) {
      transport.forget?.(key)
      if (!after.notices.some((notice) => notice.key === key)) noticeTargets.delete(key)
    }
    for (const notice of after.notices) if (!frame.some((alert) => alert.id === noticeTargets.get(notice.key)?.id &&
      alert.revision === noticeTargets.get(notice.key)?.revision && alert.state === 'open')) {
      yield* mutations.dismissNotice(notice.key)
      noticeTargets.delete(notice.key)
    }
  })
  const port: AlertActionPort = {
    act: (request) => Effect.runPromise(Effect.gen(function* () {
      const current = yield* SubscriptionRef.get(mutations.state)
      for (const notice of current.notices) if (noticeTargets.get(notice.key)?.id === request.alert.id) {
        yield* mutations.dismissNotice(notice.key)
        noticeTargets.delete(notice.key)
      }
      const previous = [...current.pending.values()].find((entry) => entry.action.alert.id === request.alert.id)
      const key = previous?.key ?? `action/${crypto.randomUUID()}`
      noticeTargets.set(key, request.alert)
      if (previous !== undefined) yield* mutations.retry(key)
      else yield* mutations.submit({ key, action: request, refusal: request.action.type === 'message.read' ||
        (request.action.type === 'work.done' && ['dismiss', 'read'].includes(request.action.answer?.id ?? '')) ? 'rollback' : 'retain' })
      const state = yield* SubscriptionRef.get(mutations.state)
      const failure = state.pending.get(key)?.failure ?? state.notices.find((notice) => notice.key === key)?.failure
      const result: AlertActionResult = failure === undefined ? { _tag: 'Sent' } : {
        _tag: 'Refused', detail: failure._tag === 'MutationRefused' ? failure.message : `Already answered by ${failure.by}.`,
      }
      if (!state.pending.has(key)) transport.forget?.(key)
      return result
    })),
  }
  return {
    port, state: mutations.state, accept,
    project: (frame: readonly Native.Attention[], pending: ReadonlyMap<string, PendingMutation<Answer>>, notices: readonly MutationNotice[] = []) => {
      let projected: readonly OptimisticAttention[] = frame
      for (const entry of pending.values()) projected = project(projected, entry)
      for (const notice of notices) projected = projected.map((alert) => noticeTargets.get(notice.key)?.id !== alert.id ? alert : {
        ...alert, optimisticNote: notice.failure._tag === 'MutationRefused' ? notice.failure.message : `Already answered by ${notice.failure.by}.`,
      })
      return projected
    },
  }
}
