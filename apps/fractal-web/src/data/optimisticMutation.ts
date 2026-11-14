import { Effect, Schema, SubscriptionRef } from 'effect'

export class MutationRefused extends Schema.TaggedError<MutationRefused>()('MutationRefused', {
  message: Schema.String,
}) {}
export class AlreadyAnswered extends Schema.TaggedError<AlreadyAnswered>()('AlreadyAnswered', {
  by: Schema.String,
}) {}
export type MutationFailure = MutationRefused | AlreadyAnswered
export interface Mutation<TAction> {
  readonly key: string
  readonly action: TAction
  readonly refusal: 'rollback' | 'retain'
}
export interface PendingMutation<TAction> extends Mutation<TAction> {
  readonly phase: 'sending' | 'accepted' | 'failed'
  readonly failure?: MutationRefused
}
export type Reconciliation = 'pending' | 'applied' | AlreadyAnswered
export interface MutationNotice {
  readonly key: string
  readonly failure: MutationFailure
}
export interface MutationState<TFrame, TAction> {
  readonly frame: TFrame
  readonly pending: ReadonlyMap<string, PendingMutation<TAction>>
  readonly notices: readonly MutationNotice[]
}

/** ACKs never remove overlays: only an authoritative frame can prove application.
 * Callers supply immutable projections and reconcile only against authoritative data,
 * not the optimistic view. Frames must arrive in authoritative order.
 */
export const makeOptimisticMutations = <TFrame, TAction, TRequirements = never>(options: {
  readonly initial: TFrame
  readonly send: (mutation: Mutation<TAction>) => Effect.Effect<void, MutationFailure, TRequirements>
  readonly project: (frame: TFrame, mutation: PendingMutation<TAction>) => TFrame
  readonly reconcile: (frame: TFrame, mutation: Mutation<TAction>) => Reconciliation
  readonly inverse: (action: TAction) => TAction
  readonly nextKey: () => string
}) => Effect.gen(function* () {
  const state = yield* SubscriptionRef.make<MutationState<TFrame, TAction>>({
    frame: options.initial, pending: new Map(), notices: [],
  })
  const deliver = Effect.fn('OptimisticMutation.deliver')(function* (entry: PendingMutation<TAction>) {
    const result = yield* options.send(entry).pipe(Effect.result)
    yield* SubscriptionRef.update(state, (current) => {
      // A frame may settle the mutation while its request is still in flight.
      if (current.pending.get(entry.key) !== entry) return current
      const pending = new Map(current.pending)
      let notices = current.notices
      if (result._tag === 'Success') pending.set(entry.key, { ...entry, phase: 'accepted' })
      else if (result.failure._tag === 'MutationRefused' && entry.refusal === 'retain') {
        pending.set(entry.key, { ...entry, phase: 'failed', failure: result.failure })
      } else {
        pending.delete(entry.key)
        notices = [...notices, { key: entry.key, failure: result.failure }]
      }
      return { ...current, pending, notices }
    })
  })
  const interrupted = (entry: PendingMutation<TAction>) => SubscriptionRef.update(state, (current) => {
    if (current.pending.get(entry.key) !== entry) return current
    const pending = new Map(current.pending)
    // Cancellation cannot prove non-delivery, even for normally rolled-back actions.
    pending.set(entry.key, { ...entry, phase: 'failed', failure: new MutationRefused({
      message: 'Delivery was interrupted; retry to confirm the outcome.',
    }) })
    return { ...current, pending }
  })
  const submit = Effect.fn('OptimisticMutation.submit')((mutation: Mutation<TAction>) => Effect.uninterruptibleMask((restore) => Effect.gen(function* () {
    const entry: PendingMutation<TAction> = { ...mutation, phase: 'sending' }
    const admitted = yield* SubscriptionRef.modify(state, (current) => {
      if (current.pending.has(entry.key)) return [false, current] as const
      const pending = new Map(current.pending)
      pending.set(entry.key, entry)
      return [true, { ...current, pending }] as const
    })
    if (admitted) yield* restore(deliver(entry)).pipe(Effect.onInterrupt(() => interrupted(entry)))
  })))
  const retry = Effect.fn('OptimisticMutation.retry')((key: string) => Effect.uninterruptibleMask((restore) => Effect.gen(function* () {
    const entry = yield* SubscriptionRef.modify(state, (current) => {
      const failed = current.pending.get(key)
      if (failed?.phase !== 'failed') return [undefined, current] as const
      const next: PendingMutation<TAction> = { key, action: failed.action, refusal: failed.refusal, phase: 'sending' }
      const pending = new Map(current.pending)
      pending.set(key, next)
      return [next, { ...current, pending }] as const
    })
    if (entry !== undefined) yield* restore(deliver(entry)).pipe(Effect.onInterrupt(() => interrupted(entry)))
  })))
  const accept = Effect.fn('OptimisticMutation.accept')((frame: TFrame) =>
    SubscriptionRef.update(state, (current) => {
      const pending = new Map(current.pending)
      const notices = [...current.notices]
      for (const [key, entry] of pending) {
        const verdict = options.reconcile(frame, entry)
        if (verdict === 'pending') continue
        pending.delete(key)
        if (verdict !== 'applied') notices.push({ key, failure: verdict })
      }
      return { frame, pending, notices }
    }),
  )
  // Undo is a new, immediately submitted command, never a delayed/cancelled send.
  const undo = Effect.fn('OptimisticMutation.undo')((mutation: Mutation<TAction>) => submit({
    key: options.nextKey(), action: options.inverse(mutation.action), refusal: mutation.refusal,
  }))
  const view = SubscriptionRef.get(state).pipe(Effect.map(({ frame, pending }) => {
    for (const entry of pending.values()) frame = options.project(frame, entry)
    return frame
  }))
  const dismissNotice = Effect.fn('OptimisticMutation.dismissNotice')((key: string) =>
    SubscriptionRef.update(state, (current) => ({ ...current, notices: current.notices.filter((notice) => notice.key !== key) })),
  )
  return { state, view, submit, retry, accept, undo, dismissNotice } as const
}).pipe(Effect.withSpan('OptimisticMutation.make'))
