import type { DevbarSegment } from '@overeng/devbar'
import { makeSeries, makeSource, runInterval } from '@overeng/meters'
import type { Meters, NumberValue, Series, SeriesView, Source, SourceRegistration, UnavailableReason } from '@overeng/meters'
import { block, fitText, historyX, tileBlock } from '@overeng/meters/canvas'
import type { CanvasBlockSpec, MeterTheme } from '@overeng/meters/canvas'
import { MeterTile } from '@overeng/meters/react'
import * as stylex from '@stylexjs/stylex'
import { Effect } from 'effect'
import * as React from 'react'

import { getDebug } from './measurement/index.ts'
import { readRequestLog, readSocketObservation, subscribeRequestLog, subscribeSocketObservation } from './transport.ts'
import type { RequestRecord, SocketObservation } from './transport.ts'

/** What one sampling pass observed; an instrument that has not measured never reads as zero. */
export type TransportReading<TValue> =
  | { readonly _tag: 'Value'; readonly value: TValue }
  | { readonly _tag: 'Unavailable'; readonly reason: UnavailableReason }

/** Used subscription slots against the limit current at that sample; the limit can change at runtime. */
export interface SubscriptionUsage {
  readonly used: number
  readonly cap: number
}

type ReadDebug = (key: string) => number
const number = (value: number): TransportReading<NumberValue> => ({ _tag: 'Value', value: { _tag: 'Number', value } })
const unavailable = (reason: UnavailableReason) => ({ _tag: 'Unavailable', reason }) as const

const httpReading = (read: ReadDebug, key: string): TransportReading<NumberValue> =>
  read('Wf.httpInstrumented') === 1 ? number(read(key)) : unavailable('NotConfigured')

const latencyReading = (read: ReadDebug, key: string): TransportReading<NumberValue> => {
  if (read('Wf.httpInstrumented') !== 1) return unavailable('NotConfigured')
  return read('Wf.httpSamples') > 0 ? number(read(key)) : unavailable('NoSamples')
}

/** Pure readings over the debug bag, the one place the live source's writers record transport. */
export const readTransport = {
  active: (read: ReadDebug) => httpReading(read, 'Wf.httpInFlight'),
  errors: (read: ReadDebug) => httpReading(read, 'Wf.httpErrors'),
  p50: (read: ReadDebug) => latencyReading(read, 'Wf.httpP50Ms'),
  p95: (read: ReadDebug) => latencyReading(read, 'Wf.httpP95Ms'),
  subscriptions: (read: ReadDebug): TransportReading<SubscriptionUsage> => {
    // The live source always configures a positive limit, so zero means it has not started.
    const cap = read('Wf.followCap')
    return cap > 0 ? { _tag: 'Value', value: { used: read('Wf.activeFollows'), cap } } : unavailable('NotConfigured')
  },
  messages: (read: ReadDebug, socket: SocketObservation | undefined): TransportReading<NumberValue> => {
    if (socket === undefined) return unavailable('NotConfigured')
    // Without a socket nothing can arrive, so a rate here would be a guess rather than a measured zero.
    return socket.state._tag === 'Live' ? number(read('Wf.frames')) : unavailable('Disconnected')
  },
}

const sampleEveryMs = 500
/** 30s at 500ms, with room for the partially observed oldest bucket. */
const capacity = 120
const series = {
  active: makeSeries<NumberValue>({ id: 'wf.http.active', label: 'HTTP requests in flight', unit: 'count', capacity }),
  p50: makeSeries<NumberValue>({ id: 'wf.http.p50', label: 'HTTP latency p50', unit: 'ms', capacity }),
  p95: makeSeries<NumberValue>({ id: 'wf.http.p95', label: 'HTTP latency p95', unit: 'ms', capacity }),
  errors: makeSeries<NumberValue>({ id: 'wf.http.errors', label: 'HTTP errors since load', unit: 'count', capacity }),
  subscriptions: makeSeries<SubscriptionUsage>({ id: 'wf.subscriptions', label: 'Socket subscriptions', unit: 'count', capacity }),
  messages: makeSeries<NumberValue>({ id: 'wf.messages', label: 'WebSocket messages per second', unit: 'count', capacity }),
}

const sampled = <TValue,>(options: {
  readonly id: string
  readonly series: Series<TValue>
  readonly read: () => TransportReading<TValue>
}): Source<TValue> =>
  makeSource({
    id: options.id,
    series: options.series,
    cadence: { _tag: 'Interval', everyMs: sampleEveryMs },
    start: ({ sink, clock }) =>
      runInterval({
        clock,
        everyMs: sampleEveryMs,
        observe: Effect.sync(() => {
          sink.append({ sample: { ...options.read(), atMs: clock.now() } })
        }),
      }),
  })

/** Host-owned sampling of real instrumentation; the kit's gauges start at a real zero, which p50/p95 cannot claim. */
export const transportSources = (): readonly SourceRegistration[] => [
  sampled({ id: 'http-active', series: series.active, read: () => readTransport.active(getDebug) }),
  sampled({ id: 'http-p50', series: series.p50, read: () => readTransport.p50(getDebug) }),
  sampled({ id: 'http-p95', series: series.p95, read: () => readTransport.p95(getDebug) }),
  sampled({ id: 'http-errors', series: series.errors, read: () => readTransport.errors(getDebug) }),
  sampled({ id: 'subscriptions', series: series.subscriptions, read: () => readTransport.subscriptions(getDebug) }),
  sampled({ id: 'message-rate', series: series.messages, read: () => readTransport.messages(getDebug, readSocketObservation()) }),
]

export const tileHistoryMs = 30_000
const perSecond = { _tag: 'Bucket', bucketMs: 1000, buckets: tileHistoryMs / 1000 } as const

/**
 * A tile whose value is not one number on one unit: `tileBlock` formats through the unit alone,
 * so it can draw neither `4/16` nor a limit known only at runtime. Candidate for upstreaming.
 */
export const textTileBlock = <TValue,>(options: {
  readonly id: string
  readonly series: Series<TValue>
  readonly shortLabel: string
  readonly widthPx: number
  readonly value: (value: TValue) => number
  /** Fixed ceiling of the plot, drawn as a mark; without one the window's maximum scales it. */
  readonly ceiling?: (value: TValue) => number
  readonly text: (value: TValue) => string
  readonly describe: (value: TValue) => string
  readonly tone: (value: TValue, theme: MeterTheme) => string
}): CanvasBlockSpec => {
  const plot = (input: {
    readonly ctx: CanvasRenderingContext2D
    readonly chart: { readonly x: number; readonly y: number; readonly width: number; readonly height: number }
    readonly history: SeriesView<TValue>
    readonly nowMs: number
    readonly color: string
    readonly ceiling: number | undefined
  }): void => {
    const { ctx, chart, history, nowMs } = input
    const startMs = nowMs - tileHistoryMs
    let top = Math.max(1, input.ceiling ?? 1)
    for (let index = 0; index < history.length; index++) {
      const sample = history.at(index)
      if (sample?._tag === 'Value' && sample.atMs >= startMs) top = Math.max(top, options.value(sample.value))
    }
    ctx.beginPath()
    let open = false
    for (let index = 0; index < history.length; index++) {
      const sample = history.at(index)
      if (sample === undefined || sample.atMs < startMs) continue
      if (sample._tag !== 'Value') {
        open = false
        continue
      }
      const x = historyX({ atMs: sample.atMs, nowMs, historyMs: tileHistoryMs, rect: chart })
      const y = chart.y + chart.height * (1 - options.value(sample.value) / top) - 0.5
      if (open) ctx.lineTo(x, y)
      else ctx.moveTo(x, y)
      open = true
    }
    ctx.strokeStyle = input.color
    ctx.lineWidth = 1.5
    ctx.stroke()
  }
  return block({
    id: options.id,
    series: options.series,
    shortLabel: options.shortLabel,
    widthPx: options.widthPx,
    describe: ({ history }) => {
      const latest = history.latest
      if (latest === undefined) return 'n/a (NoSamples)'
      if (latest._tag === 'Value') return options.describe(latest.value)
      return latest._tag === 'Gap' ? `n/a (${latest.reason} gap)` : `n/a (${latest.reason})`
    },
    draw: ({ ctx, rect, history, nowMs, theme }) => {
      ctx.save()
      ctx.beginPath()
      ctx.rect(rect.x, rect.y, rect.width, rect.height)
      ctx.clip()
      ctx.fillStyle = theme.background
      ctx.fillRect(rect.x, rect.y, rect.width, rect.height)
      const chart = { x: rect.x + 4, y: rect.y + 18, width: rect.width - 8, height: Math.max(1, rect.height - 20) }
      const latest = history.latest
      const measured = latest?._tag === 'Value' ? latest.value : undefined
      const ceiling = measured === undefined ? undefined : options.ceiling?.(measured)
      ctx.fillStyle = theme.border
      ctx.fillRect(chart.x, chart.y + chart.height - 1, chart.width, 1)
      if (ceiling !== undefined) ctx.fillRect(chart.x, chart.y, chart.width, 1)
      const color = measured === undefined ? theme.muted : options.tone(measured, theme)
      plot({ ctx, chart, history, nowMs, color, ceiling })
      ctx.font = theme.font
      ctx.textBaseline = 'top'
      ctx.textAlign = 'left'
      const measure = (text: string): number => ctx.measureText(text).width
      const left = rect.x + 4
      const right = rect.x + rect.width - 4
      const label = fitText({ text: options.shortLabel, maxWidth: right - left, measure })
      if (label !== undefined) {
        ctx.fillStyle = theme.muted
        ctx.fillText(label.text, left, rect.y + 3)
      }
      // As in tileBlock, the label is the accessible name, so a clipped label leaves no value.
      if (label?.text === options.shortLabel) {
        const value = fitText({
          text: measured === undefined ? 'n/a' : options.text(measured),
          maxWidth: right - (left + label.width + 6),
          measure,
        })
        if (value !== undefined) {
          ctx.fillStyle = measured === undefined ? theme.muted : color === theme.normal ? theme.foreground : color
          ctx.fillText(value.text, right - value.width, rect.y + 3)
        }
      }
      ctx.restore()
    },
  })
}

const value = (sample: NumberValue): number => sample.value
const latencyTile = (options: { readonly id: string; readonly series: Series<NumberValue>; readonly shortLabel: string; readonly widthPx: number }) =>
  tileBlock({ ...options, aggregation: { ...perSecond, mode: 'Gauge', unit: 'ms' }, value, range: 'auto', nominalMax: 100 })

const transportBlocks = (width: (compact: number) => number) => ({
  'http-active': tileBlock({
    id: 'http-active', series: series.active, shortLabel: 'http', widthPx: width(64),
    aggregation: { ...perSecond, mode: 'Gauge', unit: 'count' }, value, range: 'auto', nominalMax: 4,
  }),
  'http-p50': latencyTile({ id: 'http-p50', series: series.p50, shortLabel: 'p50', widthPx: width(72) }),
  'http-p95': latencyTile({ id: 'http-p95', series: series.p95, shortLabel: 'p95', widthPx: width(72) }),
  'http-errors': textTileBlock({
    id: 'http-errors', series: series.errors, shortLabel: 'err', widthPx: width(64), value,
    text: (sample) => String(sample.value),
    describe: (sample) => `${sample.value} since load`,
    tone: (sample, theme) => sample.value > 0 ? theme.danger : theme.normal,
  }),
  subscriptions: textTileBlock({
    id: 'subscriptions', series: series.subscriptions, shortLabel: 'subs', widthPx: width(76),
    value: (usage) => usage.used,
    ceiling: (usage) => usage.cap,
    text: (usage) => `${usage.used}/${usage.cap}`,
    describe: (usage) => `${usage.used} of ${usage.cap} used`,
    tone: (usage, theme) => usage.used >= usage.cap ? theme.warning : theme.normal,
  }),
  'message-rate': tileBlock({
    id: 'message-rate', series: series.messages, shortLabel: 'msg', widthPx: width(80),
    aggregation: { ...perSecond, mode: 'Counter', unit: 'count/s' }, value, range: 'auto', nominalMax: 4,
  }),
})
const compact = transportBlocks((compactWidth) => compactWidth)
const detail = transportBlocks(() => 180)
export const transportDetailBlocks: readonly CanvasBlockSpec[] = Object.values(detail)

export const transportMeterDescriptions: Readonly<Record<string, string>> = {
  websocket: 'Connection state as the SDK reports it, not a measure: n/a until a live source reports. Socket errors, resyncs and retries are in Counters.',
  'http-active': 'HTTP requests in flight through the instrumented SDK fetch, sampled twice a second, so a shorter request can fall between samples. n/a until the live source installs the instrumentation.',
  'http-p50': 'Median time to response headers over the latest 256 responses, not a time window. Cancelled and failed requests are excluded. n/a until the first response.',
  'http-p95': '95th percentile time to response headers over the latest 256 responses, not a time window. Cancelled and failed requests are excluded. n/a until the first response.',
  'http-errors': 'HTTP errors since this page loaded: non-2xx responses and network failures, not user cancellations. The number never falls; the plot shows when it rose.',
  subscriptions: 'Socket subscription slots admitted locally against the current limit. A slot is counted before server admission, so this is not a server acknowledgement.',
  'message-rate': 'WebSocket frames received per second, from the SDK frame count over completed one-second buckets. Shows offline while the socket is not live, because nothing can arrive.',
}

const styles = stylex.create({
  chip: {
    display: 'inline-flex',
    alignItems: 'center',
    gap: 5,
    height: 24,
    paddingBlock: 0,
    paddingInline: 4,
    borderWidth: 0,
    borderRadius: 4,
    backgroundColor: 'transparent',
    color: 'inherit',
    cursor: 'pointer',
    whiteSpace: 'nowrap',
    fontFamily: 'ui-monospace, monospace',
    fontSize: 11,
  },
  dot: (color: string) => ({ width: 7, height: 7, borderRadius: '50%', flexShrink: 0, backgroundColor: color }),
  tone: (color: string) => ({ color }),
  details: { marginTop: 0, marginBottom: 10, lineHeight: 1.5 },
})

type SocketTone = 'normal' | 'warning' | 'danger' | 'muted'
/** A state with words of its own, so color is never the only signal. */
export const describeSocket = (observation: SocketObservation | undefined): {
  readonly label: string
  readonly tone: SocketTone
  readonly detail: string
} => {
  if (observation === undefined) return { label: 'n/a', tone: 'muted', detail: 'No live source has reported a connection state' }
  const { state } = observation
  switch (state._tag) {
    case 'Idle': return { label: 'idle', tone: 'muted', detail: 'Not connected because nothing needs the socket' }
    case 'Connecting': return { label: 'connecting', tone: 'warning', detail: `Connecting, attempt ${state.attempt}` }
    case 'Live': return { label: 'live', tone: 'normal', detail: 'Connected and live' }
    case 'Reconnecting': return { label: 'reconnecting', tone: 'warning', detail: `Reconnecting, attempt ${state.attempt}: ${state.issue}` }
    case 'Rejected': return { label: 'rejected', tone: 'danger', detail: `Rejected by the gateway: ${state.message}` }
    case 'Closed': return { label: 'down', tone: 'danger', detail: 'Connection closed' }
  }
}

const useSocketObservation = (): SocketObservation | undefined =>
  React.useSyncExternalStore(subscribeSocketObservation, readSocketObservation, readSocketObservation)

/** Presentation of one socket state; a state element rather than a meter, because it is not a measure. */
export const SocketStateChip = ({ observation, theme, onOpen }: {
  readonly observation: SocketObservation | undefined
  readonly theme: MeterTheme
  readonly onOpen?: () => void
}) => {
  const { label, tone, detail } = describeSocket(observation)
  const color = theme[tone]
  return (
    <button type="button" data-wf-socket-state={observation?.state._tag ?? 'Unobserved'} aria-label={`WebSocket ${label}`} title={detail} onClick={onOpen} {...stylex.props(styles.chip)}>
      <span aria-hidden="true" {...stylex.props(styles.dot(color))} />
      <span>WS <span {...stylex.props(tone !== 'normal' && styles.tone(color))}>{label}</span></span>
    </button>
  )
}

const SocketState = (props: { readonly theme: MeterTheme; readonly onOpen: () => void }) => (
  <SocketStateChip observation={useSocketObservation()} {...props} />
)

/** The Meters panel's view of the state element: what it is now and since when. */
export const SocketDetails = () => {
  const observation = useSocketObservation()
  const { detail } = describeSocket(observation)
  return (
    <p {...stylex.props(styles.details)}>
      {detail}.{observation === undefined ? '' : ` Since ${new Date(observation.sinceMs).toLocaleTimeString()}.`}
    </p>
  )
}

const requestLine = ({ method, path, outcome, durationMs }: RequestRecord): string =>
  `${outcome === 'network' ? 'failed' : outcome}  ${(durationMs / 1000).toFixed(2)}s  ${method} ${path}`

/** What the numbers are made of: an error count or a p95 alone cannot say which route to look at. */
export const RequestDetails = () => {
  const { errors, slowest } = React.useSyncExternalStore(subscribeRequestLog, readRequestLog, readRequestLog)
  return (
    <pre aria-label="Recent HTTP errors and slowest responses" {...stylex.props(styles.details)}>
      {`Latest errors\n${errors.map(requestLine).join('\n') || 'none'}\n\nSlowest responses since load\n${slowest.map(requestLine).join('\n') || 'none'}`}
    </pre>
  )
}

const tileHeightPx = 28

/** Devbar segments, each with a higher `priority` staying longer; all stay below the changelog. */
export const transportSegments = ({ meters, theme, onOpenDetail }: {
  readonly meters: Pick<Meters, 'store' | 'clock'>
  readonly theme: MeterTheme
  readonly onOpenDetail: (id: string) => void
}): readonly DevbarSegment[] => {
  const tile = (id: keyof typeof compact) => () => (
    // The kit tile owns hover and focus evidence; a click also reaches the panel, as a strip meter's does.
    <div data-wf-transport-meter={id} onClick={() => onOpenDetail(id)}>
      <MeterTile meters={meters} block={compact[id]} theme={theme} heightPx={tileHeightPx} />
    </div>
  )
  return [
    { id: 'websocket', label: 'WebSocket', priority: 0, render: () => <SocketState theme={theme} onOpen={() => onOpenDetail('websocket')} /> },
    { id: 'http-active', label: 'HTTP in flight', priority: -6, render: tile('http-active') },
    { id: 'http-p50', label: 'HTTP p50', priority: -5, render: tile('http-p50') },
    { id: 'http-p95', label: 'HTTP p95', priority: -2, render: tile('http-p95') },
    { id: 'http-errors', label: 'HTTP errors', priority: -1, render: tile('http-errors') },
    { id: 'subscriptions', label: 'Subscriptions', priority: -3, render: tile('subscriptions') },
    { id: 'message-rate', label: 'Messages per second', priority: -4, render: tile('message-rate') },
  ]
}
