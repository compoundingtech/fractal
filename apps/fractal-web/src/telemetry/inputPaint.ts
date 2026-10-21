import { afterNextPaint } from './ux.ts'

export const inputPaintSpanName = 'fractal.input.next_frame'
export const inputPaintAttributeNames = ['input.kind', 'input.action', 'surface', 'outcome', 'accepted_to_frame_ms', 'read_to_frame_ms'] as const
export type InputSurface = 'agents' | 'conversation' | 'tabs' | 'palette' | 'missions' | 'other'
type Outcome = 'painted' | 'coalesced' | 'unacknowledged'
interface Input {
  readonly type: string
  readonly start: number
  readonly accepted: number
  readonly kind: 'key' | 'mouse' | 'paste' | 'focus' | 'resize' | 'input'
  readonly action: string
  readonly surface: InputSurface
  readonly owner: Element
  committed: number | undefined
  interactionId: number | undefined
  cancel: (() => void) | undefined
}
export interface InputPaintSample {
  readonly name: typeof inputPaintSpanName
  readonly startMs: number
  readonly endMs: number
  readonly attributes: Readonly<Record<string, string | number>>
}
const events = ['pointerdown', 'click', 'keydown', 'paste', 'focusin', 'focusout', 'resize', 'input'] as const
// A control owns only its DOM, not sibling content, its React root or the page.
const inputOwner = (target: EventTarget | null): Element | null => target instanceof Element
  ? target.closest('button,a[href],input,textarea,select,[contenteditable="true"],[role="row"][data-sidebar-tree-key],[role="button"],[role="tab"],[role="menuitem"],[role="menuitemcheckbox"],[role="menuitemradio"],[role="checkbox"],[role="radio"],[role="switch"],[role="option"],[role="combobox"],[role="spinbutton"],[role="slider"],[role="textbox"],[role="searchbox"]')
  : null
const surface = (target: EventTarget | null): InputSurface => {
  if (!(target instanceof Element)) return 'other'
  if (target.closest('[role="dialog"][aria-label="Command palette"]')) return 'palette'
  if (target.closest('[role="tab"],[role="tablist"]')) return 'tabs'
  if (target.closest('[role="treegrid"]')) return 'agents'
  if (target.closest('[data-conversation-entry-id],[role="region"][aria-label="Conversation history"]')) return 'conversation'
  if (target.closest('[data-perf-target="missions-list"]')) return 'missions'
  return 'other'
}

export interface InputPaintRegion { readonly target: Element; readonly subtree?: boolean }
const ownedRegions = new Map<Element, () => ReadonlyArray<InputPaintRegion>>()
const regionCommits = new Set<(owner: Element) => void>()

/** App-side surface ownership; resolve only that surface's small field/tail at input capture. */
export const registerInputPaintRegions = (owner: Element, regions: () => ReadonlyArray<InputPaintRegion>): (() => void) => {
  ownedRegions.set(owner, regions)
  return () => { if (ownedRegions.get(owner) === regions) ownedRegions.delete(owner) }
}

/** A surface's mounted DOM tail changed; not a render/Profiler notification. */
export const inputPaintRegionMounted = (owner: Element): void => {
  for (const commit of regionCommits) commit(owner)
}

const frameBoundaries = new Set<() => void>()

/** Drain input-owned records, then disconnect BEFORE stream writes or their React work.
 * Closing rather than temporarily suspending also excludes async commits caused by ingest.
 * This is needed even when ingest's rAF was registered before input capture.
 */
export const beforeInputPaintFrameIngest = (): void => {
  for (const close of frameBoundaries) close()
}

const nextTask = (callback: () => void): (() => void) => {
  const channel = new MessageChannel()
  const cancel = () => { channel.port1.close(); channel.port2.close() }
  channel.port1.onmessage = () => { cancel(); callback() }
  channel.port2.postMessage(undefined)
  return cancel
}

/**
 * TUI read_at -> event.timeStamp; accepted_at -> capture-listener entry; terminal flush ->
 * first paint following an observed DOM mutation. Thus read_to_frame_ms includes browser
 * dispatch queueing and accepted_to_frame_ms begins at capture. queue_ms is omitted: the
 * capture listener does not observe the browser's handler-entry scheduling boundary.
 * No React Profiler dependency (production React disables it). Commit ownership is the
 * control, its aria-controls/owns targets, and explicitly registered field/tail regions.
 * Overlay hosts are childList-only. No page/root/history subtree observation occurs.
 * Effects outside those regions do not count. Trusted field input is a native edit signal.
 * React 19.2.3 flushes discrete work in microtasks (ReactFiberRootScheduler.js:
 * 265-267,345-346,671-692). Completion listeners on the registered React root run AFTER
 * React's native listeners, in bubble or (if cancelBubble is set) capture phase.
 * stopPropagation does not skip same-node listeners. Their queued microtask drains after
 * React's flush and before timers/rendering. Window bubble covers non-React targets.
 * No rAF fallback: MessageChannel is only a last resort for native propagation stops below
 * the registered root or stopImmediatePropagation; those unsupported paths can include
 * intervening tasks. makeFrameIngest also explicitly drains before its writes.
 * No timeout expires committed inputs: lifecycle termination ends them unacknowledged.
 * Event Timing is bound one-to-one by event type and exact timestamp correlation before
 * retaining its interactionId; ambiguous correlations use the paint fallback, never a
 * different input's presentation. The existing rAF + posted-task boundary handles fallback.
 */
export const installInputPaint = ({ emit, now = () => performance.now(), paint = afterNextPaint, reactRoots = [], overlayHosts = [], task = nextTask }: {
  readonly emit: (sample: InputPaintSample) => void
  readonly now?: () => number
  readonly paint?: (callback: () => void) => () => void
  /** Register containers only AFTER createRoot has installed React's native listeners. */
  readonly reactRoots?: Iterable<Element>
  readonly overlayHosts?: Iterable<Element>
  readonly task?: (callback: () => void) => () => void
}): (() => void) => {
  const pending = new Set<Input>()
  let candidate: Input | undefined
  let cancelBoundary: (() => void) | undefined
  let disposed = false
  const finish = (input: Input, outcome: Outcome, end = now()) => {
    if (!pending.delete(input)) return
    input.cancel?.()
    if (candidate === input) closeWindow()
    if (input.committed === undefined) return
    emit({ name: inputPaintSpanName, startMs: input.start, endMs: Math.max(end, input.start), attributes: {
      'span.label': 'input.next_frame', 'input.kind': input.kind, 'input.action': input.action,
      surface: input.surface, outcome,
      ...(outcome === 'painted' ? {
        accepted_to_frame_ms: Math.max(0, end - input.accepted),
        read_to_frame_ms: Math.max(0, end - input.start),
      } : {}),
    } })
  }
  const observer = new MutationObserver((records) => commit(records))
  const commit = (records: ReadonlyArray<MutationRecord>) => {
    if (candidate === undefined) return
    if (!records.some((record) => {
      if (record.type === 'attributes') return record.target instanceof Element && record.oldValue !== record.target.getAttribute(record.attributeName ?? '')
      if (record.type === 'characterData') return record.oldValue !== record.target.nodeValue
      return record.addedNodes.length > 0 || record.removedNodes.length > 0
    })) return
    markCommitted()
  }
  const markCommitted = () => {
    const input = candidate
    if (input === undefined || !pending.has(input)) return
    for (const previous of pending) if (previous !== input && previous.committed !== undefined) finish(previous, 'coalesced')
    if (input.committed !== undefined) return
    input.committed = now()
    input.cancel?.()
    input.cancel = paint(() => finish(input, 'painted'))
  }
  const closeWindow = () => {
    if (candidate === undefined) return
    commit(observer.takeRecords())
    observer.disconnect()
    cancelBoundary?.()
    cancelBoundary = undefined
    if (candidate?.committed === undefined && candidate !== undefined) pending.delete(candidate)
    candidate = undefined
  }
  frameBoundaries.add(closeWindow)
  const mounted = (owner: Element) => { if (ownedRegions.has(owner) && candidate !== undefined && owner.contains(candidate.owner)) markCommitted() }
  regionCommits.add(mounted)
  const capture = (event: Event) => {
    if (disposed || !event.isTrusted || document.visibilityState === 'hidden') return
    closeWindow()
    const control = inputOwner(event.target)
    const owner = control ?? (event.target instanceof Element ? event.target : document.body)
    const accepted = now()
    const input: Input = {
      type: event.type,
      start: event.timeStamp > 1e12 ? event.timeStamp - performance.timeOrigin : event.timeStamp,
      accepted, owner, surface: surface(event.target),
      kind: event.type === 'keydown' ? 'key' : event.type === 'paste' ? 'paste' : event.type.startsWith('focus') ? 'focus' : event.type === 'resize' ? 'resize' : event.type === 'input' ? 'input' : 'mouse',
      action: event.type === 'keydown' ? 'key' : event.type.startsWith('focus') ? (event.type === 'focusin' ? 'gained' : 'lost') : event.type,
      committed: undefined, interactionId: undefined, cancel: undefined,
    }
    candidate = input
    pending.add(input)
    const roots = [...reactRoots]
    const observeRegion = (target: Element, subtree = true) => {
      if (target === document.documentElement || target === document.body) return
      // Completion roots must never become subtree observations.
      if (roots.includes(target)) return
      if (target.matches('[data-testid="transcript-scroll"],[data-testid="conversation-history-host"]') || target.querySelector('[data-testid="transcript-scroll"]') !== null) return
      observer.observe(target, { subtree, childList: true, characterData: subtree, characterDataOldValue: subtree, attributes: subtree, attributeOldValue: subtree })
    }
    if (control !== null) observeRegion(control)
    for (const key of ['aria-controls', 'aria-owns']) {
      for (const id of (owner.getAttribute(key) ?? '').split(/\s+/)) {
        if (id === '') continue
        const target = document.getElementById(id)
        if (target !== null) observeRegion(target)
      }
    }
    for (const [regionOwner, regions] of ownedRegions) {
      if (regionOwner.contains(owner)) for (const region of regions()) observeRegion(region.target, region.subtree)
    }
    for (const host of overlayHosts) {
      if (host !== document.body && host !== document.documentElement) observer.observe(host, { childList: true })
    }
    // A trusted editing event is itself the browser's field DOM/value commit.
    if (event.type === 'input' && (owner instanceof HTMLInputElement || owner instanceof HTMLTextAreaElement || owner.getAttribute('contenteditable') === 'true')) markCommitted()
    const closeThisInput = () => { if (candidate === input) closeWindow() }
    const bubble = (dispatched: Event) => {
      if (dispatched === event) queueMicrotask(closeThisInput)
    }
    const stoppedCapture = (dispatched: Event) => {
      if (dispatched === event && dispatched.cancelBubble) queueMicrotask(closeThisInput)
    }
    for (const root of roots) {
      root.addEventListener(event.type, stoppedCapture, true)
      root.addEventListener(event.type, bubble)
    }
    window.addEventListener(event.type, bubble)
    const cancelTask = task(closeThisInput)
    cancelBoundary = () => {
      for (const root of roots) {
        root.removeEventListener(event.type, stoppedCapture, true)
        root.removeEventListener(event.type, bubble)
      }
      window.removeEventListener(event.type, bubble)
      cancelTask()
    }
  }
  let eventObserver: PerformanceObserver | undefined
  const handleEventEntries = (entries: PerformanceEntryList) => {
    for (const entry of entries) {
      if (!('interactionId' in entry) || typeof entry.interactionId !== 'number' || entry.interactionId === 0) continue
      const matches = [...pending].filter((input) => entry.name === input.type && Math.abs(entry.startTime - input.start) < 0.001)
      if (matches.length !== 1) continue
      const input = matches[0]!
      if (input.interactionId !== undefined && input.interactionId !== entry.interactionId) continue
      input.interactionId = entry.interactionId
      const end = entry.startTime + entry.duration
      if (input.committed !== undefined && end >= input.committed && end <= now()) finish(input, 'painted', end)
    }
  }
  if (typeof PerformanceObserver !== 'undefined' && PerformanceObserver.supportedEntryTypes.includes('event')) {
    eventObserver = new PerformanceObserver((list) => handleEventEntries(list.getEntries()))
    eventObserver.observe({ type: 'event', durationThreshold: 16 } as PerformanceObserverInit)
  }
  const abandon = () => {
    closeWindow()
    handleEventEntries(eventObserver?.takeRecords() ?? [])
    for (const input of pending) finish(input, 'unacknowledged')
  }
  const visibility = () => { if (document.visibilityState === 'hidden') abandon() }
  for (const type of events) window.addEventListener(type, capture, true)
  window.addEventListener('pagehide', abandon, true)
  window.addEventListener('unload', abandon, true)
  document.addEventListener('visibilitychange', visibility, true)
  return () => {
    if (disposed) return
    abandon()
    disposed = true
    frameBoundaries.delete(closeWindow)
    regionCommits.delete(mounted)
    observer.disconnect()
    eventObserver?.disconnect()
    for (const type of events) window.removeEventListener(type, capture, true)
    window.removeEventListener('pagehide', abandon, true)
    window.removeEventListener('unload', abandon, true)
    document.removeEventListener('visibilitychange', visibility, true)
  }
}
