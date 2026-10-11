import { makeDetailsFixture } from './fixtures'
import type { DetailsState } from './model'
import type { TraceSeries } from './trace-series'
import { traceForDetailsState } from './trace/fixture-provider'

/** Explicit host observations for CSF stories; production components never import this module. */
export function detailsStoryInputs(state: DetailsState, trace?: TraceSeries) {
  const pending = state === 'loading' || state === 'error'
  const fixture = makeDetailsFixture(pending ? 'loading' : state)
  const populated = makeDetailsFixture('busy')
  return {
    overview: fixture.overview,
    sections: pending ? populated.sections : fixture.sections,
    trace: trace ?? traceForDetailsState(pending ? 'loading' : state),
    now: 1800000000000,
    observedAt: 1799999999000,
    pendingOverview: populated.overview,
    pendingTrace: traceForDetailsState('busy'),
  }
}
