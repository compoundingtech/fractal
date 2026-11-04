/** Compatibility entry: read core plus HTTP mutations and PTY services. */
export * from './read.ts'
export * from './mutations.ts'
export * from './pty.ts'
import * as Context from 'effect/Context'
import * as Effect from 'effect/Effect'
import * as Layer from 'effect/Layer'
import * as Read from './read.ts'
import { makeMutations, type MutationPort } from './mutations.ts'

export class Fractal extends Context.Service<
  Fractal,
  Read.Fractal['Service'] & MutationPort
>()('@fractal/sdk/Fractal') {}

/** The complete SDK retains one scoped collections socket and the original service tag. */
export const FractalLive = (options: Read.FractalOptions) => Layer.effect(Fractal,
  Effect.gen(function* () {
    const context = yield* Layer.build(Read.FractalLive(options))
    return Fractal.of({ ...Context.get(context, Read.Fractal), ...makeMutations(options) })
  }),
)
