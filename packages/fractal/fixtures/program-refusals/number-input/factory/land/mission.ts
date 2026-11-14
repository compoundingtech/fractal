import { Effect, Schema } from 'effect'

import { Fractal } from '../../../../../src/effect.ts'
import factory from '../agent.ts'

/** A number input does not encode to a native text input. */
export default Fractal.mission(
  {
    meta: import.meta,
    id: 'factory/land',
    goal: 'Land.',
    reportTo: factory,
    // @ts-expect-error a number does not encode to a native text input
    input: Schema.Struct({ attempts: Schema.Number }),
  },
  () => Effect.void,
)
