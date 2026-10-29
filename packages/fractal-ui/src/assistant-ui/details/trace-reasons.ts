import type { TraceFact, TraceUnknownReason } from './trace-series.ts'

/** Exhaustive user-facing copy for the portable trace contract; never paint its wire codes. */
const reasonText = {
  'no-provider': 'This harness does not report it.',
  'not-covered': 'This information is not tracked for this session.',
  'not-attributed': 'This information couldn’t be linked to this session.',
  'out-of-retention': 'This information is no longer retained.',
  loading: 'This information is still loading.',
  failed: 'This information could not be loaded.',
} satisfies Record<TraceUnknownReason, string>

/** A wire code, including a future unknown code, always resolves to safe prose. */
export const traceReasonText = (reason: string): string =>
  Object.hasOwn(reasonText, reason) ? reasonText[reason as TraceUnknownReason] : 'Not reported'

/** Host-supplied prose is retained; unknown wire codes have a safe, nontechnical fallback. */
export function detailsReasonText(reason: string): string {
  const value = reason.trim()
  if (Object.hasOwn(reasonText, value)) return reasonText[value as TraceUnknownReason]
  return value.length === 0 || /^[a-z]+(?:-[a-z]+)+$/.test(value) ? 'Not reported' : value
}

export const traceFactReason = (fact: TraceFact<unknown>): string =>
  fact._tag === 'Unknown' ? fact.detail === undefined ? traceReasonText(fact.reason) : detailsReasonText(fact.detail) : 'Not reported'

/** Several absent inputs can share the same explanation, e.g. all three occupancy inputs. */
export const joinDetailsReasons = (reasons: readonly string[]): string =>
  [...new Set(reasons.map(detailsReasonText))].join(' ')
