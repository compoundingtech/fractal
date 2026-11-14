import { Effect, Schema } from 'effect'

import { Fractal } from '../../../../../src/effect.ts'
import factory from '../agent.ts'

/** Declares an id that is not its directory. */
export default Fractal.mission(
  {
    meta: import.meta,
    id: 'factory/elsewhere',
    goal: 'Land.',
    reportTo: factory,
    input: Schema.Struct({ change: Schema.String }),
  },
  () => Effect.void,
)
