import type * as Native from '@fractal/client/schema'
import { Effect, Option, SubscriptionRef } from 'effect'
import { makeOptimisticMutations, MutationRefused, type MutationNotice, type PendingMutation } from '../../data/optimisticMutation.ts'
import type { AlertAction, AlertActionPort, AlertActionResult, AlertAnswered } from './alert-action-port.ts'

type Answer = { readonly alert: AlertAnswered; readonly action: AlertAction; readonly supersedes?: string }
export type OptimisticAttention = Native.Attention & {
  readonly optimisticAnswer?: { readonly phase: 'sending' | 'accepted' | 'failed'; readonly detail?: string; readonly action: AlertAction }
  readonly optimisticNote?: string
}
const project = (frame: readonly OptimisticAttention[], entry: PendingMutation<Answer>): readonly OptimisticAttention[] =>
  entry.refusal === 'rollback' && entry.phase !== 'failed' && entry.action.action.type !== 'attention.restore' &&
    !(entry.action.action.type === 'attention.snooze' && entry.action.action.until === null)
    ? frame.filter((alert) => alert.id !== entry.action.alert.id)
    : frame.map((alert) => alert.id !== entry.action.alert.id ? alert : {
      ...alert, optimisticAnswer: { phase: entry.phase, detail: entry.failure?.message, action: entry.action.action },
    })

/** One connection owns the answer overlays; every inbox and conversation sees the same state. */
export const optimisticAlertActions = (transport: AlertActionPort) => {
  const noticeTargets = new Map<string, AlertAnswered>()
  const receipts = new Map<string, Extract<AlertActionResult, { _tag: 'Sent' }> & { readonly supersedes?: string }>()
  const mutations = Effect.runSync(makeOptimisticMutations<readonly Native.Attention[], Answer>({
    initial: [], nextKey: () => `action/${crypto.randomUUID()}`,
    send: ({ key, action }) => Effect.tryPromise({
      try: () => transport.act({ ...action, key }),
      catch: () => new MutationRefused({ message: 'The answer could not be confirmed. Retry to confirm it.' }),
    }).pipe(Effect.flatMap((result) => {
      if (result._tag === 'Refused') return Effect.fail(new MutationRefused({ message: result.detail }))
      receipts.set(key, { ...result, ...(action.supersedes === undefined ? {} : { supersedes: action.supersedes }) })
      return Effect.void
    })),
    project,
    reconcile: (frame, mutation) => {
      const row = frame.find((alert) => alert.id === mutation.action.alert.id)
      const { alert, action } = mutation.action
      if (action.type === 'attention.dismiss' || action.type === 'attention.restore' || action.type === 'attention.snooze') {
        const control = row?.presentation_control
        // An inverse's authoritative receipt also proves its predecessor was applied.
        if (control?.source_revision === alert.presentation_control?.source_revision && control !== undefined &&
          [...receipts.values()].some((receipt) => receipt.supersedes === mutation.key &&
            receipt.controlRevision === Option.getOrNull(control.revision))) return 'applied'
        const revision = receipts.get(mutation.key)?.controlRevision
        if (control === undefined || revision === undefined || Option.getOrNull(control.revision) !== revision ||
          control.source_revision !== alert.presentation_control?.source_revision) return 'pending'
        return (action.type === 'attention.snooze'
          ? Option.getOrNull(control.snoozed_until) === action.until && control.dismissed === alert.presentation_control?.dismissed
          : control.dismissed === (action.type === 'attention.dismiss') &&
            Option.getOrNull(control.snoozed_until) === Option.getOrNull(alert.presentation_control!.snoozed_until)) ? 'applied' : 'pending'
      }
      return row === undefined || row.state === 'resolved' || row.revision !== mutation.action.alert.revision ? 'applied' : 'pending'
    },
  }))
  const accept = Effect.fn('OptimisticAlertActions.accept')(function* (frame: readonly Native.Attention[]) {
    const before = yield* SubscriptionRef.get(mutations.state)
    yield* mutations.accept(frame)
    const after = yield* SubscriptionRef.get(mutations.state)
    for (const key of before.pending.keys()) if (!after.pending.has(key)) {
      transport.forget?.(key)
      receipts.delete(key)
      if (!after.notices.some((notice) => notice.key === key)) noticeTargets.delete(key)
    }
    for (const notice of after.notices) if (frame.some((alert) => alert.id === noticeTargets.get(notice.key)?.id &&
      (alert.revision !== noticeTargets.get(notice.key)?.revision || alert.state !== 'open'))) {
      yield* mutations.dismissNotice(notice.key)
      noticeTargets.delete(notice.key)
    }
  })
  const port: { readonly act: (request: Answer & { readonly key?: string }) => Promise<AlertActionResult> } = {
    act: (request) => Effect.runPromise(Effect.gen(function* () {
      const current = yield* SubscriptionRef.get(mutations.state)
      for (const notice of current.notices) if (noticeTargets.get(notice.key)?.id === request.alert.id) {
        yield* mutations.dismissNotice(notice.key)
        noticeTargets.delete(notice.key)
      }
      const previous = request.key === undefined
        ? [...current.pending.values()].find((entry) => entry.action.alert.id === request.alert.id) : current.pending.get(request.key)
      const key = request.key ?? previous?.key ?? `action/${crypto.randomUUID()}`
      noticeTargets.set(key, request.alert)
      if (previous !== undefined) yield* mutations.retry(key)
      else yield* mutations.submit({ key, action: request, refusal: request.action.type.startsWith('attention.') ||
        request.action.type === 'message.read' ||
        (request.action.type === 'work.done' && ['dismiss', 'read'].includes(request.action.answer?.id ?? '')) ? 'rollback' : 'retain' })
      const state = yield* SubscriptionRef.get(mutations.state)
      const failure = state.pending.get(key)?.failure ?? state.notices.find((notice) => notice.key === key)?.failure
      const result: AlertActionResult = failure === undefined ? { _tag: 'Sent' } : {
        _tag: 'Refused', detail: failure._tag === 'MutationRefused' ? failure.message : `Already answered by ${failure.by}.`,
      }
      const receipt = receipts.get(key)
      const control = request.alert.presentation_control
      const action = request.action
      if (!state.pending.has(key)) { transport.forget?.(key); receipts.delete(key) }
      if (result._tag === 'Sent' && receipt?.controlRevision !== undefined && control !== undefined &&
        (action.type === 'attention.dismiss' || action.type === 'attention.restore' || action.type === 'attention.snooze')) {
        const inverse: AlertAction = action.type === 'attention.snooze'
          ? { type: 'attention.snooze', until: Option.getOrNull(control.snoozed_until) }
          : { type: action.type === 'attention.dismiss' ? 'attention.restore' : 'attention.dismiss' }
        return { ...result, undo: () => port.act({
          key: `action/${crypto.randomUUID()}`, supersedes: key,
          alert: { ...request.alert, presentation_control: {
            ...control, revision: Option.fromNullishOr(receipt.controlRevision),
            dismissed: action.type === 'attention.snooze' ? control.dismissed : action.type === 'attention.dismiss',
            snoozed_until: action.type === 'attention.snooze' ? Option.fromNullishOr(action.until) : control.snoozed_until,
          } },
          action: inverse,
        }) }
      }
      return result
    })),
  }
  return {
    port, state: mutations.state, accept,
    project: (frame: readonly Native.Attention[], pending: ReadonlyMap<string, PendingMutation<Answer>>, notices: readonly MutationNotice[] = []) => {
      let projected: readonly OptimisticAttention[] = frame
      const superseded = new Set([...pending.values()].map((entry) => entry.action.supersedes))
      for (const entry of pending.values()) if (!superseded.has(entry.key)) projected = project(projected, entry)
      for (const notice of notices) projected = projected.map((alert) => noticeTargets.get(notice.key)?.id !== alert.id ? alert : {
        ...alert, optimisticNote: notice.failure._tag === 'MutationRefused' ? notice.failure.message : `Already answered by ${notice.failure.by}.`,
      })
      return projected
    },
  }
}
