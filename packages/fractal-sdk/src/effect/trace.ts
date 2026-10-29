/** Platform-neutral W3C trace context validation and span propagation. */
import type { TraceContext } from '@fractal/client'
import type * as Tracer from 'effect/Tracer'

export type { TraceContext } from '@fractal/client'

const TRACEPARENT = /^00-([0-9a-f]{32})-([0-9a-f]{16})-[0-9a-f]{2}$/

/** Lowercase W3C version 00 with non-zero trace and span ids, as the generated client accepts. */
export const isValidTraceparent = (value: string): boolean => {
  const match = TRACEPARENT.exec(value)
  return match !== null && !/^0+$/.test(match[1]!) && !/^0+$/.test(match[2]!)
}

/** The `traceparent` naming `span` as the remote parent. */
export const traceparentOf = (span: Tracer.AnySpan): string =>
  `00-${span.traceId}-${span.spanId}-${span.sampled ? '01' : '00'}`

/** The context naming `span` as the remote parent. */
export const traceContextOf = (span: Tracer.AnySpan): TraceContext => ({ traceparent: traceparentOf(span) })

/** The callback's context when valid; an invalid `traceparent` suppresses `tracestate` too. */
export const validTraceContext = (
  traceContext: (() => TraceContext | undefined) | undefined,
): TraceContext | undefined => {
  const context = traceContext?.()
  if (context === undefined || !isValidTraceparent(context.traceparent)) return undefined
  return context.tracestate === undefined
    ? { traceparent: context.traceparent }
    : { traceparent: context.traceparent, tracestate: context.tracestate }
}

