import { FractalPlatform } from '@fractal/sdk/effect'
import { makeAdmission } from '@fractal/sdk/admission'
import { Layer } from 'effect'

import { makeVisibilityPolicy } from './admission.ts'
import { takeEarlyCollections } from './early.ts'
import { browserSocket, traceQuerySocket } from './trace.ts'

/** Browser-only transport and pane admission policy, explicitly provided to the SDK layer. */
export const BrowserFractalPlatform = ({
  adoptEarlyCollections = true,
}: {
  readonly adoptEarlyCollections?: boolean
} = {}): Layer.Layer<FractalPlatform> => Layer.succeed(FractalPlatform, {
  socket: traceQuerySocket(browserSocket),
  takeEarlyCollections: adoptEarlyCollections ? takeEarlyCollections : undefined,
  admission: (options) => makeAdmission({ ...options, policy: makeVisibilityPolicy<string>() }),
})
