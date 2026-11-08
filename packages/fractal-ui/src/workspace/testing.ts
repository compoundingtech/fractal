import { Arbitrary, Effect, type Scope } from 'effect'
import * as TestClock from 'effect/testing/TestClock'
import { it } from 'vitest'

/**
 * A property test over an Effect `Arbitrary`, with shrinking: the property throws (an `expect`) to fail,
 * and the shrunk input is run once more so the report carries the assertion itself. The seed is fixed so
 * a run is the same on every machine; widen `runs` or change `seed` locally to search further.
 */
export const prop = <A>(name: string, arbitrary: Arbitrary.Arbitrary<A>, property: (value: A) => void, options?: { readonly runs?: number; readonly seed?: number }) =>
  it(name, async () => {
    const holds = (value: A) => {
      try {
        property(value)
        return true
      } catch {
        return false
      }
    }
    const result = await Effect.runPromise(Arbitrary.checkEffect(arbitrary, holds, { runs: options?.runs ?? 100, seed: options?.seed ?? 20261108 }))
    if (result._tag === 'Passed') return
    if (result._tag === 'Falsified') {
      try {
        property(result.shrunkInput)
      } catch (error) {
        throw new Error(`${Arbitrary.formatCheckFailure(result) ?? 'Falsified'}\n${String(error)}`, { cause: error })
      }
    }
    throw new Error(Arbitrary.formatCheckFailure(result) ?? result._tag)
  }, 60_000)

/** An Effect test under a test clock: `TestClock.adjust` moves debounces along. */
export const itEffect = <E>(name: string, program: () => Effect.Effect<void, E, Scope.Scope>) =>
  it(name, () => Effect.runPromise(program().pipe(Effect.scoped, Effect.provide(TestClock.layer()))))
