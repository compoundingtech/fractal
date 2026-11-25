import { Effect, Schema } from 'effect'

import { Fractal } from '../src/effect.ts'

export default Fractal.mission(
  {
    meta: { url: import.meta.url },
    id: 'test/cancel-finalizers',
    finalizerGrace: '5m',
    goal: 'Run a journaled finalizer command when interrupted.',
    reportTo: { _tag: 'SubjectRef', kind: 'agent', url: 'file:///test/operator.ts' },
    input: Schema.Struct({ hang: Schema.String }),
  },
  ({ hang }) => Fractal.step({ id: 'work' },
    Fractal.command(['sh', '-c', 'sleep 1000']).pipe(Effect.orDie),
  ).pipe(Effect.ensuring(
    Fractal.step({ id: 'drain', success: Schema.String },
      hang === 'yes'
        ? Effect.never
        : Fractal.command(['sh', '-c', hang === 'fail' ? 'exit 7' : 'printf drained']).pipe(
          Effect.flatMap((result) => result.code === 0 ? Effect.succeed(result.stdout) : Effect.die('drain failed')),
          Effect.orDie,
        ),
    ).pipe(Effect.orDie),
  )),
)
