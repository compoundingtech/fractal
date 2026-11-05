import { Effect, Schema } from 'effect'
import type * as Pty from '@fractal/sdk/pty'
import type * as Mutations from '@fractal/sdk/mutations'
import { createLazyChunk } from '@fractal/ui/lazy-chunk'

/** A failed import is forgotten so the existing action/pane retry can acquire it again. */
export class SdkChunkFailure extends Schema.TaggedError<SdkChunkFailure>()('SdkChunkFailure', {
  message: Schema.String,
}) {}

export const retryableImport = <T>(load: () => Promise<T>, label: string): Effect.Effect<T, SdkChunkFailure> => {
  const chunk = createLazyChunk({ load })
  let failed = false
  return Effect.tryPromise({
    try: () => (!failed ? chunk.load()
      : chunk.recovery === 'reload' ? chunk.reload().then(chunk.load)
      : chunk.retry()).then((module) => { failed = false; return module }),
    catch: (cause) => {
      failed = true
      return new SdkChunkFailure({ message: `Could not load ${label}. Try again. ${String(cause)}` })
    },
  })
}

export const loadPty = retryableImport<typeof Pty>(() => import('@fractal/sdk/pty'), 'terminal services')
export const loadMutations = retryableImport<typeof Mutations>(() => import('@fractal/sdk/mutations'), 'editing services')
