import { it } from '@effect/vitest'
import { Deferred, Effect, Fiber, SubscriptionRef } from 'effect'
import { expect } from 'vitest'
import { AlreadyAnswered, makeOptimisticMutations, MutationRefused, type Mutation, type MutationFailure } from './optimisticMutation.ts'

type Frame = { readonly value: number; readonly applied: readonly string[]; readonly answeredBy?: string }
const initial: Frame = { value: 0, applied: [] }
const command: Mutation<number> = { key: 'first', action: 1, refusal: 'retain' }
const make = (send: (mutation: Mutation<number>) => Effect.Effect<void, MutationFailure>) =>
  makeOptimisticMutations({
    initial, send, nextKey: () => 'inverse', inverse: (action) => -action,
    project: (frame, mutation) => ({ ...frame, value: frame.value + mutation.action }),
    reconcile: (frame, mutation) => frame.applied.includes(mutation.key) ? 'applied'
      : frame.answeredBy === undefined ? 'pending' : new AlreadyAnswered({ by: frame.answeredBy }),
  })

it.effect('publishes before sending; ACK waits for the authoritative frame', () => Effect.gen(function* () {
  const started = yield* Deferred.make<void>()
  const finish = yield* Deferred.make<void>()
  const mutations = yield* make(() => Effect.gen(function* () {
    yield* Deferred.succeed(started, undefined)
    yield* Deferred.await(finish)
  }))
  const sending = yield* mutations.submit(command).pipe(Effect.forkChild)
  yield* Deferred.await(started)
  expect((yield* mutations.view).value).toBe(1)
  expect((yield* SubscriptionRef.get(mutations.state)).pending.get('first')?.phase).toBe('sending')
  expect(yield* mutations.discardFailed('first')).toBe(false)
  yield* mutations.submit(command)
  yield* Deferred.succeed(finish, undefined)
  yield* Fiber.join(sending)
  expect((yield* SubscriptionRef.get(mutations.state)).pending.get('first')?.phase).toBe('accepted')
  expect(yield* mutations.discardFailed('first')).toBe(false)
  yield* mutations.accept({ value: 10, applied: [] })
  expect((yield* mutations.view).value).toBe(11)
  yield* mutations.accept({ value: 11, applied: ['first'] })
  expect((yield* mutations.view).value).toBe(11)
  expect((yield* SubscriptionRef.get(mutations.state)).pending.size).toBe(0)
}))

it.effect('small refusals roll back with a dismissible note', () => Effect.gen(function* () {
  const failure = new MutationRefused({ message: 'Not permitted' })
  const mutations = yield* make(() => Effect.fail(failure))
  yield* mutations.submit({ ...command, refusal: 'rollback' })
  expect((yield* mutations.view).value).toBe(0)
  expect((yield* SubscriptionRef.get(mutations.state)).notices).toEqual([{ key: 'first', failure }])
  yield* mutations.retry('first')
  yield* mutations.dismissNotice('first')
  expect((yield* SubscriptionRef.get(mutations.state)).notices).toEqual([])
}))

it.effect('answers stay failed and retry with the same identity and action', () => Effect.gen(function* () {
  const requests: Mutation<number>[] = []
  const mutations = yield* make((mutation) => Effect.suspend(() => {
    requests.push(mutation)
    return requests.length === 1 ? Effect.fail(new MutationRefused({ message: 'Try again' })) : Effect.void
  }))
  yield* mutations.submit(command)
  expect((yield* mutations.view).value).toBe(1)
  expect((yield* SubscriptionRef.get(mutations.state)).pending.get('first')).toMatchObject({ phase: 'failed' })
  yield* mutations.retry('first')
  expect(requests.map(({ key, action }) => ({ key, action }))).toEqual([
    { key: 'first', action: 1 }, { key: 'first', action: 1 },
  ])
  yield* mutations.retry('first')
  expect(requests).toHaveLength(2)
}))

it.effect('another answer drops the local draft and identifies its author', () => Effect.gen(function* () {
  const mutations = yield* make(() => Effect.void)
  yield* mutations.submit(command)
  yield* mutations.accept({ value: 9, applied: [], answeredBy: 'another person' })
  expect((yield* mutations.view).value).toBe(9)
  expect((yield* SubscriptionRef.get(mutations.state)).notices).toMatchObject([
    { key: 'first', failure: { _tag: 'AlreadyAnswered', by: 'another person' } },
  ])
  yield* mutations.retry('first')
  expect((yield* SubscriptionRef.get(mutations.state)).pending.size).toBe(0)
  const refused = yield* make(() => Effect.fail(new AlreadyAnswered({ by: 'another person' })))
  yield* refused.submit(command)
  expect((yield* SubscriptionRef.get(refused.state)).pending.size).toBe(0)
  expect((yield* SubscriptionRef.get(refused.state)).notices[0]?.failure._tag).toBe('AlreadyAnswered')
}))

it.effect('a frame settling an in-flight action wins over a late refusal', () => Effect.gen(function* () {
  const started = yield* Deferred.make<void>()
  const finish = yield* Deferred.make<void>()
  const mutations = yield* make(() => Effect.gen(function* () {
    yield* Deferred.succeed(started, undefined)
    yield* Deferred.await(finish)
    return yield* new MutationRefused({ message: 'Late failure' })
  }))
  const sending = yield* mutations.submit(command).pipe(Effect.forkChild)
  yield* Deferred.await(started)
  yield* mutations.accept({ value: 1, applied: ['first'] })
  yield* Deferred.succeed(finish, undefined)
  yield* Fiber.join(sending)
  expect((yield* SubscriptionRef.get(mutations.state)).notices).toEqual([])
  expect((yield* mutations.view).value).toBe(1)
}))

it.effect('Undo immediately sends the inverse under a fresh key', () => Effect.gen(function* () {
  const requests: Mutation<number>[] = []
  const mutations = yield* make((mutation) => Effect.sync(() => { requests.push(mutation) }))
  yield* mutations.submit(command)
  if (mutations.undo === undefined) return yield* Effect.die('Expected an inverse action')
  yield* mutations.undo(command)
  expect(requests.map(({ key, action }) => ({ key, action }))).toEqual([
    { key: 'first', action: 1 }, { key: 'inverse', action: -1 },
  ])
  expect((yield* mutations.view).value).toBe(0)
  yield* mutations.accept({ value: 1, applied: ['first'] })
  expect((yield* mutations.view).value).toBe(0)
  yield* mutations.accept({ value: 0, applied: ['first', 'inverse'] })
  expect((yield* SubscriptionRef.get(mutations.state)).pending.size).toBe(0)
}))

it.effect('interrupted delivery remains recoverable with the same key', () => Effect.gen(function* () {
  const started = yield* Deferred.make<void>()
  let attempts = 0
  const mutations = yield* make(() => Effect.gen(function* () {
    attempts++
    if (attempts === 1) {
      yield* Deferred.succeed(started, undefined)
      yield* Effect.never
    }
  }))
  const sending = yield* mutations.submit(command).pipe(Effect.forkChild)
  yield* Deferred.await(started)
  yield* Fiber.interrupt(sending)
  expect((yield* mutations.view).value).toBe(1)
  expect((yield* SubscriptionRef.get(mutations.state)).pending.get('first')?.phase).toBe('failed')
  yield* mutations.retry('first')
  expect(attempts).toBe(2)
  expect((yield* SubscriptionRef.get(mutations.state)).pending.get('first')).toMatchObject({ key: 'first', phase: 'accepted' })
}))

it.effect('authoritative settlement wins over late success and interruption', () => Effect.gen(function* () {
  for (const interrupt of [false, true]) {
    const started = yield* Deferred.make<void>()
    const finish = yield* Deferred.make<void>()
    const mutations = yield* make(() => Effect.gen(function* () {
      yield* Deferred.succeed(started, undefined)
      yield* Deferred.await(finish)
    }))
    const sending = yield* mutations.submit(command).pipe(Effect.forkChild)
    yield* Deferred.await(started)
    yield* mutations.accept({ value: 1, applied: ['first'] })
    if (interrupt) yield* Fiber.interrupt(sending)
    else {
      yield* Deferred.succeed(finish, undefined)
      yield* Fiber.join(sending)
    }
    expect((yield* SubscriptionRef.get(mutations.state)).pending.size).toBe(0)
    expect((yield* SubscriptionRef.get(mutations.state)).notices).toEqual([])
    expect((yield* mutations.view).value).toBe(1)
  }
}))

it.effect('explicit abandonment removes only a failed intent before fresh staging', () => Effect.gen(function* () {
  const requests: string[] = []
  const mutations = yield* make(({ key }) => Effect.suspend(() => {
    requests.push(key)
    return key === 'first' ? Effect.fail(new MutationRefused({ message: 'Not applied' })) : Effect.void
  }))
  yield* mutations.submit(command)
  expect(yield* mutations.discardFailed('first')).toBe(true)
  expect((yield* mutations.view).value).toBe(0)
  yield* mutations.retry('first')
  expect(requests).toEqual(['first'])
  yield* mutations.submit({ ...command, key: 'restaged' })
  expect((yield* SubscriptionRef.get(mutations.state)).pending.get('restaged')?.phase).toBe('accepted')
  expect(yield* mutations.discardFailed('restaged')).toBe(false)
  yield* mutations.accept({ value: 1, applied: ['restaged'] })
  expect(yield* mutations.discardFailed('restaged')).toBe(false)
}))
