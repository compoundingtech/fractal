import { Effect, Schema } from 'effect'
import type * as Pty from '@fractal/sdk/pty'
import type * as Mutations from '@fractal/sdk/mutations'

/** A failed import is forgotten so the existing action/pane retry can acquire it again. */
export class SdkChunkFailure extends Schema.TaggedError<SdkChunkFailure>()('SdkChunkFailure', {
  message: Schema.String,
}) {}

export const retryableImport = <T>(load: () => Promise<T>, label: string): Effect.Effect<T, SdkChunkFailure> => {
  let pending: Promise<T> | undefined
  return Effect.tryPromise({
    try: () => pending ??= load().catch((cause: unknown) => {
      pending = undefined
      throw cause
    }),
    catch: (cause) => new SdkChunkFailure({ message: `Could not load ${label}. Try again. ${String(cause)}` }),
  })
}

export const loadPty = retryableImport<typeof Pty>(() => import('@fractal/sdk/pty'), 'terminal services')
export const loadMutations = retryableImport<typeof Mutations>(() => import('@fractal/sdk/mutations'), 'editing services')
