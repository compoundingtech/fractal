import type * as Native from '@fractal/client/schema'
import { Effect, Option, SubscriptionRef } from 'effect'
import { AlreadyAnswered, makeOptimisticMutations, MutationRefused, type MutationNotice, type PendingMutation } from '../../data/optimisticMutation.ts'
import type { AlertAction, AlertActionPort, AlertActionResult, AlertAnswered } from './alert-action-port.ts'
import type { AlertResolution } from './alert-resolution.ts'

type Answer = { readonly alert: AlertAnswered; readonly action: AlertAction; readonly supersedes?: string }
export type OptimisticAttention = Native.Attention & {
  readonly optimisticAnswer?: { readonly phase: 'sending' | 'accepted' | 'failed'; readonly detail?: string; readonly action: AlertAction }
  readonly optimisticNote?: string
  readonly optimisticResolvedBy?: string
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
  const controlProofs = new Map<string, Native.AttentionControl>()
  const resolutionProofs = new Map<string, AlertResolution>()
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
      if (result._tag === 'Refused') return Effect.fail(result.alreadyAnsweredBy === undefined
        ? new MutationRefused({ message: result.detail }) : new AlreadyAnswered({ by: result.alreadyAnsweredBy }))
      receipts.set(key, { ...result, ...(action.supersedes === undefined ? {} : { supersedes: action.supersedes }) })
      return Effect.void
    })),
    project,
    reconcile: (frame, mutation) => {
      const row = frame.find((alert) => alert.id === mutation.action.alert.id)
      if (mutation.action.action.type.startsWith('attention.'))
        return reconcileControl(controlProofs.get(mutation.action.alert.id), mutation) === 'applied'
          ? 'applied' : reconcileControl(row?.presentation_control, mutation)
      if (!receipts.has(mutation.key)) return 'pending'
      const proof = resolutionProofs.get(mutation.action.alert.id)
      return proof?._tag === 'Resolved' || (proof?._tag === 'Read' && mutation.action.action.type === 'message.read') ||
        (row?.state === 'resolved' && row.resolved_by !== undefined) ? 'applied' : 'pending'
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
    const pendingIds = new Set([...after.pending.values()].map((entry) => entry.action.alert.id))
    for (const id of controlProofs.keys()) if (!pendingIds.has(id)) controlProofs.delete(id)
    for (const id of resolutionProofs.keys()) if (!pendingIds.has(id)) resolutionProofs.delete(id)
    for (const notice of after.notices) if (frame.some((alert) => alert.id === noticeTargets.get(notice.key)?.id &&
      (alert.revision !== noticeTargets.get(notice.key)?.revision || alert.state !== 'open'))) {
      yield* mutations.dismissNotice(notice.key)
      noticeTargets.delete(notice.key)
    }
  })
  const acceptControl = Effect.fn('OptimisticAlertActions.acceptControl')(function* (control: Native.AttentionControl) {
    controlProofs.set(control.id, control)
    const current = yield* SubscriptionRef.get(mutations.state)
    // The shared primitive alone owns pending admission, settlement and delivery races.
    yield* accept(current.frame)
  })
  const acceptResolution = Effect.fn('OptimisticAlertActions.acceptResolution')(function* (alert: AlertAnswered, proof: AlertResolution) {
    resolutionProofs.set(alert.id, proof)
    const current = yield* SubscriptionRef.get(mutations.state)
    yield* accept(current.frame)
  })
  const readResolution = Effect.fn('OptimisticAlertActions.readResolution')(function* (alert: AlertAnswered) {
    if (transport.readResolution === undefined) return
    const proof = yield* Effect.tryPromise(() => transport.readResolution!(alert)).pipe(Effect.option)
    if (Option.isSome(proof) && proof.value !== undefined) yield* acceptResolution(alert, proof.value)
  })
  const readControl = Effect.fn('OptimisticAlertActions.readControl')(function* (alert: AlertAnswered) {
    const read = transport.readControl
    if (read === undefined || alert.presentation_control === undefined) return
    const proof = yield* Effect.tryPromise(() => read(alert)).pipe(Effect.option)
    // Read failure is not mutation refusal: keep the accepted overlay for explicit retry/reconnect.
    if (Option.isSome(proof)) yield* acceptControl(proof.value)
  })
  const refreshProofs = Effect.gen(function* () {
    const current = yield* SubscriptionRef.get(mutations.state)
    const answers = new Map<string, Answer>()
    for (const entry of current.pending.values()) if (entry.phase === 'accepted') answers.set(entry.action.alert.id, entry.action)
    yield* Effect.forEach(answers.values(), ({ alert, action }) =>
      action.type.startsWith('attention.') ? readControl(alert) : readResolution(alert), { discard: true })
  })
  const port: { readonly act: (request: Answer & { readonly key?: string }) => Promise<AlertActionResult> } = {
    act: (request) => Effect.runPromise(Effect.gen(function* () {
      const current = yield* SubscriptionRef.get(mutations.state)
      for (const notice of current.notices) if (noticeTargets.get(notice.key)?.id === request.alert.id) {
        yield* mutations.dismissNotice(notice.key)
        noticeTargets.delete(notice.key)
      }
      const previous = request.key === undefined
        ? [...current.pending.values()].find((entry) => entry.action.alert.id === request.alert.id &&
          entry.action.action.type === request.action.type) : current.pending.get(request.key)
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
        ...(failure._tag === 'AlreadyAnswered' ? { alreadyAnsweredBy: failure.by } : {}),
      }
      const receipt = receipts.get(key)
      const control = request.alert.presentation_control
      const action = request.action
      if (state.pending.get(key)?.phase === 'accepted')
        yield* (action.type.startsWith('attention.') ? readControl(request.alert) : readResolution(request.alert))
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
    port, state: mutations.state, accept, acceptControl, acceptResolution, refreshProofs,
    project: (frame: readonly Native.Attention[], pending: ReadonlyMap<string, PendingMutation<Answer>>, notices: readonly MutationNotice[] = []) => {
      let projected: readonly OptimisticAttention[] = frame
      const superseded = new Set([...pending.values()].map((entry) => entry.action.supersedes))
      for (const entry of pending.values()) if (!superseded.has(entry.key)) projected = project(projected, entry)
      for (const notice of notices) projected = projected.map((alert) => noticeTargets.get(notice.key)?.id !== alert.id ? alert : {
        ...alert, optimisticNote: notice.failure._tag === 'MutationRefused' ? notice.failure.message : `Already answered by ${notice.failure.by}.`,
        ...(notice.failure._tag === 'AlreadyAnswered' ? { optimisticResolvedBy: notice.failure.by } : {}),
      })
      return projected
    },
  }
}
