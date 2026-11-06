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
  const reconcileControl = (control: Native.Attention['presentation_control'], mutation: { readonly key: string; readonly action: Answer }) => {
    const { alert, action } = mutation.action
    if (action.type !== 'attention.dismiss' && action.type !== 'attention.restore' && action.type !== 'attention.snooze') return 'pending'
    // An inverse's authoritative frame also proves its predecessor was applied.
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
      if (mutation.action.action.type.startsWith('attention.')) return reconcileControl(row?.presentation_control, mutation)
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
  const acceptControl = Effect.fn('OptimisticAlertActions.acceptControl')((control: Native.AttentionControl) =>
    SubscriptionRef.update(mutations.state, (current) => {
      const applied = [...current.pending.values()].filter((entry) => entry.action.alert.id === control.id &&
        reconcileControl(control, entry) === 'applied')
      const pending = new Map(current.pending)
      for (const entry of applied) {
        pending.delete(entry.key)
        receipts.delete(entry.key)
        noticeTargets.delete(entry.key)
        transport.forget?.(entry.key)
      }
      // A proof settles only controls; it neither inserts hidden rows nor settles source answers.
      return { ...current, pending }
    }),
  )
  const readControl = Effect.fn('OptimisticAlertActions.readControl')(function* (alert: AlertAnswered) {
    const read = transport.readControl
    if (read === undefined || alert.presentation_control === undefined) return
    const proof = yield* Effect.tryPromise(() => read(alert)).pipe(Effect.option)
    // Read failure is not mutation refusal: keep the accepted overlay for explicit retry/reconnect.
    if (Option.isSome(proof)) yield* acceptControl(proof.value)
  })
  const refreshControls = Effect.gen(function* () {
    const current = yield* SubscriptionRef.get(mutations.state)
    const alerts = new Map<string, AlertAnswered>()
    for (const entry of current.pending.values()) if (entry.phase === 'accepted' &&
      entry.action.action.type.startsWith('attention.')) alerts.set(entry.action.alert.id, entry.action.alert)
    yield* Effect.forEach(alerts.values(), readControl, { discard: true })
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
      if (state.pending.get(key)?.phase === 'accepted' && action.type.startsWith('attention.'))
        yield* readControl(request.alert)
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
    port, state: mutations.state, accept, acceptControl, refreshControls,
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
