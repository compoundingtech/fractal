import { useAtomValue } from '@effect/atom-react'
import { Devbar } from '@overeng/devbar'
import type { DevbarPanel, DevbarSegment } from '@overeng/devbar'
import { darkDevbarTheme, lightDevbarTheme } from '@overeng/devbar/themes'
import { makeMeters, makeSeries } from '@overeng/meters'
import type { FpsValue } from '@overeng/meters'
import { darkMeterTheme, frameBlock, heapBlock, jankBlock, lightMeterTheme } from '@overeng/meters/canvas'
import type { CanvasBlockSpec } from '@overeng/meters/canvas'
import { makeBrowserPlatform } from '@overeng/meters/platform/browser'
import { MetersProvider, MeterStrip, MeterTile } from '@overeng/meters/react'
import { frameSource } from '@overeng/meters/sources/frame'
import { longFramesSource } from '@overeng/meters/sources/long-frames'
import type { LongFrameValue } from '@overeng/meters/sources/long-frames'
import { heapSource } from '@overeng/meters/sources/memory'
import type { HeapMemory } from '@overeng/meters/sources/memory'
import * as stylex from '@stylexjs/stylex'
import * as Atom from 'effect/reactivity/Atom'
import * as React from 'react'
import { buildIdentity, deploymentId } from 'virtual:build-identity'

import { ChangelogContent, useChangelogDisclosure, useChangelogUnseen } from '../changelog/Changelog.tsx'
import { BuildChangesSegment } from './BuildChangesSegment.tsx'
import { reserveDevbarSpace } from './devbarSpace.ts'
import { counters } from './measurement/index.ts'
import { setDevBarDefault, toggleDevBar, useDevBarVisible } from './devBarPreference.ts'
import { RequestDetails, SocketDetails, tileHistoryMs, transportDetailBlocks, transportMeterDescriptions, transportSegments, transportSources } from './transportMeters.tsx'

export const DevBar = (props: { readonly defaultVisible?: boolean }) => <DevBarContent {...props} />

const styles = stylex.create({
  root: { zIndex: 1000 },
  panel: { padding: 12, color: 'inherit', fontFamily: 'ui-monospace, monospace' },
  description: { marginTop: 0, marginBottom: 10, lineHeight: 1.5 },
  counters: { whiteSpace: 'pre-wrap', lineHeight: 1.5, fontVariantNumeric: 'tabular-nums' },
})

// Definitions are inert; the one MetersProvider owns the kit's scoped collector lease.
const frames = makeSeries<FpsValue>({ id: 'wf.frames', label: 'Frame rate', unit: 'fps', capacity: 1200 })
const longFrames = makeSeries<LongFrameValue>({ id: 'wf.longFrames', label: 'Long frames', unit: 'ms', capacity: 256 })
const memory = makeSeries<HeapMemory>({ id: 'wf.memory', label: 'JS heap (approx.)', unit: 'bytes', capacity: 120 })
const meters = makeMeters({
  platform: makeBrowserPlatform(),
  sources: [
    frameSource({ id: 'frame', series: frames }),
    longFramesSource({ id: 'long-frames', series: longFrames }),
    heapSource({ id: 'memory', series: memory, everyMs: 1000 }),
    ...transportSources(),
  ],
})
const compactBlocks = [
  frameBlock({ id: 'frame', series: frames, widthPx: 80 }),
  jankBlock({ id: 'long-frames', series: longFrames, widthPx: 90 }),
  heapBlock({ id: 'memory', series: memory, widthPx: 110 }),
]
const detailBlocks = [
  frameBlock({ id: 'frame', series: frames, widthPx: 180 }),
  jankBlock({ id: 'long-frames', series: longFrames, widthPx: 180 }),
  heapBlock({ id: 'memory', series: memory, widthPx: 180 }),
]

/** The visibility shortcut outlives panel selection and the collector lease. */
const diagnosticLifetime = Atom.make((get) => {
  if (typeof window === 'undefined') return
  const keydown = (event: KeyboardEvent) => {
    if (!event.defaultPrevented && (event.metaKey || event.ctrlKey) && event.shiftKey && event.code === 'KeyB') {
      event.preventDefault()
      toggleDevBar()
    }
  }
  window.addEventListener('keydown', keydown)
  get.addFinalizer(() => window.removeEventListener('keydown', keydown))
})

const counterSnapshot = Atom.make((get) => {
  const timer = setInterval(() => get.setSelf(counters.snapshot()), 250)
  get.addFinalizer(() => clearInterval(timer))
  return counters.snapshot()
})

const CountersPanel = () => {
  const snapshot = useAtomValue(counterSnapshot)
  const counterText = Object.entries(snapshot)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, value]) => `${key}  ${value}`)
    .join('\n') || 'No measurement counters recorded.'
  return (
    <div {...stylex.props(styles.panel)}>
      <p {...stylex.props(styles.description)}>
        Counter totals are process-local values recorded by the app measurement instrumentation.
        Transport latency uses completed SDK fetches through response headers.
      </p>
      <p {...stylex.props(styles.description)}>
        {buildIdentity.displayVersion ?? buildIdentity.machineVersion} · Machine: {buildIdentity.machineVersion} · Source:{' '}
        {buildIdentity.sourceKind} · Revision: {buildIdentity.rev ?? 'unavailable'} · Dirty:{' '}
        {buildIdentity.dirty ? 'yes' : 'no'} · Commit timestamp: {buildIdentity.commitTs ?? 'unavailable'} · Build timestamp:{' '}
        {buildIdentity.buildTs ?? 'unavailable'} · Deployment: {deploymentId ?? 'unavailable'}
      </p>
      <pre {...stylex.props(styles.counters)} aria-label="Measurement counters">{counterText}</pre>
    </div>
  )
}

const subscribeScheme = (notify: () => void): (() => void) => {
  if (typeof document === 'undefined') return () => {}
  const observer = new MutationObserver(notify)
  observer.observe(document.documentElement, { attributes: true, subtree: true, attributeFilter: ['data-scheme'] })
  return () => observer.disconnect()
}
const readDarkScheme = (): boolean => {
  if (typeof document === 'undefined') return false
  // The live workbench owns its palette locally, overriding the document's system preference.
  const host = document.querySelector<HTMLElement>('[data-testid="live-agent-workspace"]')
  return (host?.dataset.scheme ?? document.documentElement.dataset.scheme) === 'dark'
}
const readServerScheme = (): boolean => false

const MeterView = ({ blocks, dark, onOpenDetail }: {
  readonly blocks?: readonly CanvasBlockSpec[]
  readonly dark: boolean
  readonly onOpenDetail: (selection: { readonly id: string }) => void
}) => {
  const [frozen, setFrozen] = React.useState(false)
  return <MeterStrip meters={meters} blocks={blocks ?? detailBlocks}
    theme={dark ? darkMeterTheme : lightMeterTheme} frozen={frozen} onFrozenChange={setFrozen}
    onOpenDetail={onOpenDetail} heightPx={64} {...(blocks === undefined ? {} : { historyMs: tileHistoryMs })} />
}

const meterDescriptions: Readonly<Record<string, string>> = {
  frame: 'Frame rate is observed rAF timing. The kit calibrates skipped-frame evidence separately; pending or unsupported calibration is not treated as a measured zero.',
  'long-frames': 'Long-animation-frame durations use PerformanceObserver, with an explicitly tagged long-task fallback on browsers without LoAF support. NoSamples means no event has been observed, not a measured zero.',
  memory: 'JS heap is an approximate shared-heap measurement from performance.memory, not total app memory. Unsupported capabilities are shown as n/a, not zero.',
  ...transportMeterDescriptions,
}
const MetersPanel = ({ dark, detail, setDetail }: {
  readonly dark: boolean
  readonly detail: string
  readonly setDetail: (id: string) => void
}) => {
  return (
    <div {...stylex.props(styles.panel)}>
      <p {...stylex.props(styles.description)}>Frame, long-frame, memory, and transport histories share one scoped session. Click a meter for its measurement semantics; freeze each strip independently.</p>
      <MeterView dark={dark} onOpenDetail={({ id }) => setDetail(id)} />
      <MeterView blocks={transportDetailBlocks} dark={dark} onOpenDetail={({ id }) => setDetail(id)} />
      <p {...stylex.props(styles.description)}>{meterDescriptions[detail]}</p>
      {detail === 'websocket' && <SocketDetails />}
      {detail.startsWith('http-') && <RequestDetails />}
    </div>
  )
}

const DevBarContent = ({ defaultVisible = import.meta.env?.DEV ?? false }: { readonly defaultVisible?: boolean }) => {
  setDevBarDefault(defaultVisible)
  const shown = useDevBarVisible()
  useAtomValue(diagnosticLifetime)
  const dark = React.useSyncExternalStore(subscribeScheme, readDarkScheme, readServerScheme)
  const [openPanel, setOpenPanel] = React.useState<string | undefined>(undefined)
  const [detail, setDetail] = React.useState('frame')
  const openDetail = React.useCallback((id: string) => {
    setDetail(id)
    setOpenPanel('meters')
  }, [])
  const panels = React.useMemo<readonly DevbarPanel[]>(() => [
    { id: 'counters', label: 'Counters', render: () => <CountersPanel /> },
    { id: 'meters', label: 'Meters', render: () => <MetersPanel dark={dark} detail={detail} setDetail={setDetail} /> },
  ], [dark, detail])
  const hasUnseen = useChangelogUnseen()
  const changelog = useChangelogDisclosure()
  const segments = React.useMemo<readonly DevbarSegment[]>(() => [
    // Every meter is a segment, so one row flows left to right and one overflow covers them all; freezing lives in the drawer's strips.
    ...compactBlocks.map((block) => ({
      id: block.id,
      label: block.label,
      priority: 3,
      render: () => (
        <div onClick={() => openDetail(block.id)}>
          <MeterTile meters={meters} block={block} theme={dark ? darkMeterTheme : lightMeterTheme} heightPx={28} />
        </div>
      ),
    })),
    ...transportSegments({ meters, theme: dark ? darkMeterTheme : lightMeterTheme, onOpenDetail: openDetail }),
    // One build entry: identity and recent changes outlast transport detail; when overflowed, the +N control mirrors the unseen dot.
    {
      id: 'build',
      priority: 1,
      hasAlert: hasUnseen,
      render: () => (
        <BuildChangesSegment
          identity={buildIdentity}
          deploymentId={deploymentId}
          dark={dark}
          hasUnseen={hasUnseen}
          onChangesSeen={changelog.acknowledge}
          onPrefetch={changelog.prefetch}
          changes={(firstEntryRef) => <ChangelogContent firstEntryRef={firstEntryRef} />}
        />
      ),
    },
  ], [changelog, dark, hasUnseen, openDetail])
  const host = React.useRef<HTMLDivElement>(null)
  React.useLayoutEffect(() => host.current === null ? undefined : reserveDevbarSpace(host.current), [shown])
  if (!shown) return null
  return (
    <div ref={host} data-testid="wf-devbar" {...stylex.props(dark ? darkDevbarTheme : lightDevbarTheme)}>
      <MetersProvider meters={meters}>
        <Devbar panels={panels} segments={segments} openPanel={openPanel} onOpenPanelChange={setOpenPanel} stripMinWidth={0} style={styles.root} />
      </MetersProvider>
    </div>
  )
}
