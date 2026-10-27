import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { surfaceVars, textVars, borderVars, radiusVars, spaceVars, typeVars, geometryNumbers, motionNumbers } from './composition-tokens.stylex'

const rowSelector = '[data-item-id], [data-embrace-entry-id]'
/** History anchors also resolve to leaf rows inside a turn: tool calls, thoughts and work summaries. */
const anchorSelector = `${rowSelector}, [data-conversation-entry-id], [data-scroll-anchor-id]`
const rowKey = (row: HTMLElement): string | undefined => row.dataset.scrollAnchorId ?? row.dataset.itemId ?? row.dataset.embraceEntryId ?? row.dataset.conversationEntryId
/** A summary owns its source leaves even while they are folded or unmounted. */
const rowMembers = (row: HTMLElement): readonly string[] => row.dataset.scrollAnchorMembers === undefined ? [] : JSON.parse(row.dataset.scrollAnchorMembers)
/** A prefix can grow inside a turn without moving its wrapper, so the wrapper anchors only when it has no leaf row. */
const isAnchorContainer = (row: HTMLElement) => row.hasAttribute('data-scroll-anchor-container')
const navigationKeys: Readonly<Record<string, true>> = { PageUp: true, PageDown: true, Home: true, End: true, ArrowUp: true, ArrowDown: true, ' ': true }

/** A bounded lane owns scrolling; a misconfigured host may leave it to an ancestor or the page. */
function pressScrollOwner(lane: HTMLElement): HTMLElement {
  const view = lane.ownerDocument.defaultView
  for (let candidate: HTMLElement | null = lane; candidate !== null; candidate = candidate.parentElement) {
    if (candidate.scrollHeight > candidate.clientHeight && /^(auto|scroll|overlay)$/.test(view?.getComputedStyle(candidate).overflowY ?? '')) return candidate
  }
  const documentOwner = lane.ownerDocument.scrollingElement
  return documentOwner instanceof HTMLElement && documentOwner.scrollHeight > documentOwner.clientHeight ? documentOwner : lane
}

/** Scroll state a conversation keeps while it stays on a surface. */
export interface ViewportState {
  readonly top: number
  readonly following: boolean
  readonly unread: boolean
  readonly firstRenderedId?: string
  /** The reader's line by row identity: `top` alone drifts when a remount backfills rows above it. `memberId` is a source leaf the row owns, so a summary whose turn id changed still restores. */
  readonly anchor?: { readonly id: string; readonly offset: number; readonly memberId?: string }
}

/** Complete source membership, independent of which rows are mounted; only complete absence retires a saved anchor. */
export type ViewportAnchorHistory =
  | { readonly _tag: 'Complete'; readonly includes: (id: string) => boolean }
  | { readonly _tag: 'Partial' }

/** Per-surface memory, one entry per conversation. Owners drop closed keys and dispose on unmount. */
export class ViewportStore {
  private readonly states = new Map<string, ViewportState>()
  private disposed = false
  get size(): number { return this.states.size }
  get(key: string): ViewportState | undefined { return this.states.get(key) }
  save(key: string, state: ViewportState): void { if (!this.disposed) this.states.set(key, state) }
  retain(keys: ReadonlySet<string>): void { for (const key of this.states.keys()) if (!keys.has(key)) this.states.delete(key) }
  open(): void { this.disposed = false }
  /** Parent-first teardown must ignore late saves from unmounting viewports. */
  dispose(): void { this.disposed = true; this.states.clear() }
}

/** Viewports outside a store-owning surface keep memory only for their own mount. */
export const ViewportStoreContext = React.createContext<ViewportStore | undefined>(undefined)

/**
 * Eases an explicit return to the end over the standard motion duration. Ordinary following still settles before paint.
 * The live edge is re-read every frame because streaming rows keep moving it. Reduced motion, or no media-query engine,
 * returns instantly; switching to reduced motion mid-return finishes it at once.
 */
class FollowAnimation {
  private element: HTMLElement | null = null
  private frame: number | undefined
  private started: number | undefined
  private from = 0
  private motion: MediaQueryList | undefined

  constructor(private readonly writeTop: (top: number) => void) {}

  get active(): boolean { return this.element !== null }

  /** Without motion the caller's settle pins the end before paint. */
  start(element: HTMLElement) {
    this.cancel()
    const motion = element.ownerDocument.defaultView?.matchMedia?.('(prefers-reduced-motion: reduce)')
    if (motion === undefined || motion.matches) return
    this.motion = motion
    motion.addEventListener('change', this.motionChanged)
    this.element = element
    this.from = element.scrollTop
    this.started = undefined
    this.frame = requestAnimationFrame(this.step)
  }

  cancel() {
    if (this.frame !== undefined) cancelAnimationFrame(this.frame)
    this.frame = undefined
    this.motion?.removeEventListener('change', this.motionChanged)
    this.motion = undefined
    this.element = null
  }

  private readonly motionChanged = (event: MediaQueryListEvent) => {
    const element = this.element
    if (!event.matches || element === null) return
    this.cancel()
    this.writeTop(element.scrollHeight)
  }

  private readonly step = (time: number) => {
    const element = this.element
    if (element === null) return
    this.started ??= time
    const progress = Math.min(1, (time - this.started) / motionNumbers.standard)
    // Ease-out cubic: the lane starts moving at once and lands softly on the live end.
    const eased = 1 - (1 - progress) ** 3
    const end = Math.max(0, element.scrollHeight - element.clientHeight)
    this.writeTop(this.from + (end - this.from) * eased)
    if (progress < 1) this.frame = requestAnimationFrame(this.step)
    else this.cancel()
  }
}

/** Scroll and row geometry are imperative; they never invalidate the message subtree. */
class ViewportController {
  private element: HTMLDivElement | null = null
  private jumpButton: HTMLButtonElement | null = null
  private following = true
  private anchor: { element: HTMLElement; offset: number } | undefined
  /** The anchor by row identity, captured while this conversation owns the DOM. */
  private snapshotAnchor: ViewportState['anchor']
  /** A restored row, applied once it mounts; reader input or explicit navigation clears it with `pendingTop`. */
  private pendingAnchor: ViewportState['anchor']
  private frame: number | undefined
  private captureFrame: number | undefined
  private unread = false
  private lastTop = 0
  private lastWidth = 0
  private programmaticTop: number | undefined
  private restored: ViewportState | undefined
  /** A restored line whose content is settling; the first reader scroll clears it. */
  private pendingTop: number | undefined
  /** Pointers down somewhere on the page; the dock keeps its layout until all are released. */
  private readonly pressed = new Set<number>()
  private pressedAnchor: { pointerId: number; element: HTMLElement; scrollOwner: HTMLElement; top: number } | undefined
  private warnedScrollOwner = false
  private readonly followAnimation = new FollowAnimation(top => { this.writeTop(top) })
  firstRenderedId: string | undefined
  loadEarlier: (() => void) | undefined

  private loadEarlierIfNeeded() {
    const element = this.element
    if (element === null || this.loadEarlier === undefined || element.clientHeight === 0) return
    if (this.following ? element.scrollHeight >= element.clientHeight * 2 : element.scrollTop > element.clientHeight) return
    this.prepareMaterialize()
    const load = this.loadEarlier
    this.loadEarlier = undefined
    load()
  }
  /** Prepending history is not a request to stop following or to move the reader. */
  readonly prepareMaterialize = () => {
    if (this.following) return
    const moved = this.element !== null && Math.abs(this.element.scrollTop - this.lastTop) >= geometryNumbers.scrollEndTolerance
    // Chunks already preserve this anchor. Re-hit-testing every chunk forces fresh PrePaint work.
    if (this.captureFrame === undefined && !moved && this.anchor?.element.isConnected) return
    if (this.captureFrame !== undefined) cancelAnimationFrame(this.captureFrame)
    this.captureFrame = undefined
    this.capture()
  }

  /** Explicit navigation owns history, rather than fighting the tail-follow schedule. */
  readonly reveal = () => {
    this.followAnimation.cancel()
    this.following = false
    this.pendingTop = undefined
    this.pendingAnchor = undefined
    this.anchor = undefined
    if (this.frame !== undefined) cancelAnimationFrame(this.frame)
    this.frame = undefined
  }

  constructor(saved?: ViewportState) {
    if (saved !== undefined && !saved.following) {
      this.restored = saved
      this.pendingAnchor = saved.anchor
      this.snapshotAnchor = saved.anchor
      this.unread = saved.unread
    }
  }

  readonly attachJump = (button: HTMLButtonElement | null) => {
    this.jumpButton = button
    this.dock()
  }

  /** Shows the jump only for unread rows, and never reflows the lane under an active press. */
  private dock() {
    if (this.jumpButton !== null && this.pressed.size === 0) this.jumpButton.hidden = !this.unread
  }

  readonly released = (): ViewportState => {
    const state = { top: this.lastTop, following: this.following, unread: this.unread, firstRenderedId: this.firstRenderedId }
    const anchor = this.pendingAnchor ?? this.snapshotAnchor
    return this.following || anchor === undefined ? state : { ...state, anchor }
  }

  /** A pending row may still await adoption or backfill; only its absence from complete source history retires it, leaving the saved top. */
  readonly updateHistory = (history: ViewportAnchorHistory) => {
    const saved = this.pendingAnchor
    if (saved === undefined || history._tag !== 'Complete' || history.includes(saved.memberId ?? saved.id)) return
    this.pendingAnchor = undefined
    if (this.snapshotAnchor === saved) this.snapshotAnchor = undefined
  }

  /** Restores the saved row at its saved offset; false while neither that row nor a summary owning its member is mounted. */
  private restoreAnchor(): boolean {
    const element = this.element
    const saved = this.pendingAnchor
    if (element === null || saved === undefined) return false
    const member = saved.memberId ?? saved.id
    let row: HTMLElement | undefined
    for (const candidate of element.querySelectorAll<HTMLElement>(anchorSelector)) {
      if (!isAnchorContainer(candidate) && (rowKey(candidate) === member || (rowKey(candidate) === saved.id && (saved.memberId === undefined || rowMembers(candidate).includes(saved.memberId))))) { row = candidate; break }
    }
    if (row === undefined) {
      for (const candidate of element.querySelectorAll<HTMLElement>('[data-scroll-anchor-members]')) {
        if (rowMembers(candidate).includes(member)) { row = candidate; break }
      }
    }
    if (row === undefined) return false
    this.writeTop(element.scrollTop + row.getBoundingClientRect().top - element.getBoundingClientRect().top - saved.offset)
    this.anchor = { element: row, offset: saved.offset }
    const id = rowKey(row)!
    this.snapshotAnchor = rowMembers(row).includes(member)
      ? { id, offset: saved.offset, memberId: member }
      : { id, offset: saved.offset }
    this.pendingAnchor = undefined
    this.pendingTop = undefined
    return true
  }

  /** Swaps a reused viewport to another conversation without carrying its unread mark across. */
  readonly resume = (saved?: ViewportState) => {
    this.followAnimation.cancel()
    this.unread = saved !== undefined && !saved.following && saved.unread
    this.snapshotAnchor = saved?.following === false ? saved.anchor : undefined
    this.dock()
    if (saved !== undefined && !saved.following) {
      this.following = false
      this.anchor = undefined
      this.pendingAnchor = saved.anchor
      this.pendingTop = saved.top
      this.writeTop(saved.top)
      this.scheduleCapture()
      this.schedule()
    } else {
      this.following = true
      this.anchor = undefined
      this.pendingAnchor = undefined
      if (this.element !== null) this.writeTop(this.element.scrollHeight)
      this.schedule()
    }
  }

  private capture() {
    const element = this.element
    if (element === null) return
    const viewport = element.getBoundingClientRect()
    const top = viewport.top
    const hit = element.ownerDocument.elementFromPoint(viewport.left + viewport.width / 2, top + geometryNumbers.scrollEndTolerance)?.closest<HTMLElement>(anchorSelector)
    if (hit !== undefined && hit !== null && element.contains(hit) && !isAnchorContainer(hit)) {
      this.remember(hit, hit.getBoundingClientRect().top - top)
      return
    }
    let container: { row: HTMLElement; offset: number } | undefined
    for (const row of element.querySelectorAll<HTMLElement>(anchorSelector)) {
      const bounds = row.getBoundingClientRect()
      if (bounds.bottom <= top) continue
      if (!isAnchorContainer(row)) {
        this.remember(row, bounds.top - top)
        return
      }
      container ??= { row, offset: bounds.top - top }
    }
    if (container !== undefined) this.remember(container.row, container.offset)
  }
  private remember(row: HTMLElement, offset: number) {
    this.anchor = { element: row, offset }
    const id = rowKey(row)
    if (id === undefined) { this.snapshotAnchor = undefined; return }
    const members = rowMembers(row)
    // Preserve source identity when a leaf folds into a summary, not just across summary recaptures.
    const previousMember = this.snapshotAnchor?.memberId ?? this.snapshotAnchor?.id
    const memberId = previousMember !== undefined && members.includes(previousMember) ? previousMember : members[0]
    this.snapshotAnchor = memberId === undefined ? { id, offset } : { id, offset, memberId }
  }
  private scheduleCapture() {
    if (this.captureFrame !== undefined) return
    this.captureFrame = requestAnimationFrame(() => {
      this.captureFrame = undefined
      // A pending restore still owns the reader's line; capturing first would save the pre-restore row.
      if (!this.following && this.pendingAnchor === undefined) this.capture()
    })
  }


  private writeTop(top: number) {
    const element = this.element
    if (element === null) return
    const target = Math.max(0, Math.min(top, element.scrollHeight - element.clientHeight))
    if (Math.abs(target - element.scrollTop) < geometryNumbers.scrollEndTolerance) return
    element.scrollTop = target
    this.programmaticTop = element.scrollTop
    this.lastTop = element.scrollTop
  }

  /** Pin the pressed row, not the first visible row: insertion above must not move its action. */
  readonly preservePress = () => {
    const viewport = this.element
    const anchor = this.pressedAnchor
    if (viewport === null || anchor === undefined || !anchor.element.isConnected) return false
    const delta = anchor.element.getBoundingClientRect().top - anchor.top
    if (anchor.scrollOwner === viewport) this.writeTop(viewport.scrollTop + delta)
    else anchor.scrollOwner.scrollTop += delta
    // Hand history back in the compensated coordinate system; a stale offset would undo the press scroll.
    if (this.anchor?.element.isConnected) this.remember(this.anchor.element, this.anchor.element.getBoundingClientRect().top - viewport.getBoundingClientRect().top)
    return true
  }

  /** Commit one window boundary before paint, using the existing following/anchor policy. */
  readonly settle = () => {
    const element = this.element
    if (element === null) return
    if (this.preservePress()) { this.lastWidth = element.clientWidth; return }
    if (this.following) {
      // An eased return re-reads the live end every frame; snapping here would cut it short.
      if (!this.followAnimation.active) this.writeTop(element.scrollHeight)
    } else if (this.restoreAnchor()) this.scheduleCapture()
    else if (this.pendingTop !== undefined) {
      this.writeTop(this.pendingTop)
      if (Math.abs(element.scrollTop - Math.max(0, Math.min(this.pendingTop, element.scrollHeight - element.clientHeight))) < geometryNumbers.scrollEndTolerance) {
        this.pendingTop = undefined
        this.scheduleCapture()
      }
    } else if (this.captureFrame === undefined && this.anchor?.element.isConnected) {
      this.writeTop(element.scrollTop + this.anchor.element.getBoundingClientRect().top - element.getBoundingClientRect().top - this.anchor.offset)
    }
    this.lastWidth = element.clientWidth
  }

  readonly schedule = () => {
    if (this.frame !== undefined) return
    this.frame = requestAnimationFrame(() => {
      this.frame = undefined
      this.settle()
      this.loadEarlierIfNeeded()
    })
  }

  readonly jump = () => {
    const returning = !this.following
    this.following = true
    this.unread = false
    this.anchor = undefined
    this.pendingTop = undefined
    this.pendingAnchor = undefined
    this.snapshotAnchor = undefined
    this.dock()
    // Only a return from history eases; an already-following lane keeps settling before paint.
    if (returning && this.element !== null) this.followAnimation.start(this.element)
    this.schedule()
  }

  readonly changed = () => {
    if (!this.following) {
      this.unread = true
      this.dock()
    }
    this.schedule()
  }

  readonly attach = (element: HTMLDivElement | null) => {
    if (element === null) return
    this.element = element
    this.lastWidth = element.clientWidth
    const manual = (event: Event) => {
      // Focusing the lane itself does not navigate to a row or change the reader's position.
      if (event.type === 'focusin' && event.target === element) return
      if (event instanceof KeyboardEvent && (navigationKeys[event.key] !== true || (event.target instanceof HTMLElement && event.target.closest('input,textarea,[contenteditable="true"]')))) return
      if (event.type === 'wheel' || event.type === 'touchmove' || event.type === 'keydown') this.pressedAnchor = undefined
      // Reader input interrupts an eased return where it is.
      this.followAnimation.cancel()
      this.following = false
      this.programmaticTop = undefined
      this.pendingTop = undefined
      this.pendingAnchor = undefined
      this.scheduleCapture()
      if (this.frame !== undefined) cancelAnimationFrame(this.frame)
      this.frame = undefined
    }
    const scroll = () => {
      if (this.programmaticTop !== undefined && Math.abs(element.scrollTop - this.programmaticTop) < geometryNumbers.scrollEndTolerance) {
        this.programmaticTop = undefined
        return
      }
      // Reflow is not a request to follow. Width changes preserve the reader's anchor.
      if (element.clientWidth !== this.lastWidth) { this.schedule(); return }
      if (Math.abs(element.scrollTop - this.lastTop) < geometryNumbers.scrollEndTolerance) return
      if (this.pendingTop !== undefined) {
        this.writeTop(this.pendingTop)
        return
      }
      // A real reader scroll (including scrollbar dragging) takes ownership until the next press, and interrupts an eased return.
      this.followAnimation.cancel()
      this.pressedAnchor = undefined
      this.lastTop = element.scrollTop
      this.following = element.scrollHeight - element.clientHeight - element.scrollTop <= geometryNumbers.scrollEndTolerance
      if (this.following) {
        this.unread = false
        this.dock()
        this.anchor = undefined
      } else this.scheduleCapture()
      this.loadEarlierIfNeeded()
    }
    // Document-wide, so presses that start anywhere (a row action included) defer the reveal.
    const page = element.ownerDocument
    const view = page.defaultView
    const press = (event: PointerEvent) => {
      this.pressed.add(event.pointerId)
      if (this.pressedAnchor !== undefined || !(event.target instanceof Element)) return
      const row = event.target.closest<HTMLElement>(rowSelector)
      if (row !== null && element.contains(row)) {
        const scrollOwner = pressScrollOwner(element)
        if (scrollOwner !== element && !this.warnedScrollOwner && process.env.NODE_ENV !== 'production') {
          this.warnedScrollOwner = true
          console.warn('EmbraceScrollViewport: the lane is not its own scroll container. Bound its height so following and history anchoring stay lane-local; held row actions use the actual scroll owner.')
        }
        this.pressedAnchor = { pointerId: event.pointerId, element: row, scrollOwner, top: row.getBoundingClientRect().top }
      }
    }
    const release = (event: PointerEvent) => {
      this.pressed.delete(event.pointerId)
      if (this.pressedAnchor?.pointerId === event.pointerId) this.pressedAnchor = undefined
      this.dock()
    }
    // A release the page never sees (the window blurs, the tab hides) must not latch the dock.
    const abandon = () => {
      this.pressed.clear()
      this.pressedAnchor = undefined
      this.dock()
    }
    const hidden = () => { if (page.visibilityState === 'hidden') abandon() }
    const observer = new ResizeObserver(() => { this.preservePress(); this.schedule() })
    observer.observe(element)
    if (element.firstElementChild !== null) observer.observe(element.firstElementChild)
    element.addEventListener('wheel', manual, { passive: true })
    element.addEventListener('touchmove', manual, { passive: true })
    element.addEventListener('keydown', manual)
    element.addEventListener('pointerdown', manual, { passive: true })
    element.addEventListener('focusin', manual)
    element.addEventListener('scroll', scroll, { passive: true })
    page.addEventListener('pointerdown', press, true)
    page.addEventListener('pointerup', release, true)
    page.addEventListener('pointercancel', release, true)
    page.addEventListener('lostpointercapture', release, true)
    page.addEventListener('visibilitychange', hidden)
    view?.addEventListener('blur', abandon)
    if (this.restored !== undefined) {
      this.following = false
      this.pendingTop = this.restored.top
      this.writeTop(this.restored.top)
      this.lastTop = this.restored.top
      this.scheduleCapture()
      this.restored = undefined
    }
    this.schedule()
    return () => {
      observer.disconnect()
      this.followAnimation.cancel()
      if (this.frame !== undefined) cancelAnimationFrame(this.frame)
      this.frame = undefined
      if (this.captureFrame !== undefined) cancelAnimationFrame(this.captureFrame)
      this.captureFrame = undefined
      element.removeEventListener('wheel', manual)
      element.removeEventListener('touchmove', manual)
      element.removeEventListener('keydown', manual)
      element.removeEventListener('pointerdown', manual)
      element.removeEventListener('focusin', manual)
      element.removeEventListener('scroll', scroll)
      page.removeEventListener('pointerdown', press, true)
      page.removeEventListener('pointerup', release, true)
      page.removeEventListener('pointercancel', release, true)
      page.removeEventListener('lostpointercapture', release, true)
      page.removeEventListener('visibilitychange', hidden)
      view?.removeEventListener('blur', abandon)
      this.pressed.clear()
      this.pressedAnchor = undefined
      this.element = null
    }
  }
}

/** `version` is the row's rendered content; without one, the row object itself is its version. */
export interface ViewportRow { readonly id: string; readonly version?: string }

/** A republished snapshot with the same rows is not news: compare ids and content versions, not array identity. */
function sameRows(previous: readonly ViewportRow[], next: readonly ViewportRow[]) {
  if (previous.length !== next.length) return false
  return next.every((row, index) => {
    const before = previous[index]!
    return before.id === row.id && (row.version === undefined ? before === row : before.version === row.version)
  })
}

export interface EmbraceScrollViewportProps extends Omit<React.HTMLAttributes<HTMLDivElement>, 'children'> {
  readonly items: readonly ViewportRow[]
  /** Authoritative source ids, not just mounted rows; omitted means `items` is the complete flat history. */
  readonly anchorHistory?: ViewportAnchorHistory
  readonly contentProps?: React.HTMLAttributes<HTMLDivElement>
  /** Each key keeps its own scroll state across viewport mounts. */
  readonly stateKey?: string
  /** Host command: a changed, defined key resumes following and clears unread; an unchanged key never scrolls. */
  readonly scrollToBottomKey?: string
  /** Mount the tail first, then fill one extra viewport and prepend on upward navigation. */
  readonly initialTailCount?: number
  /** Deferred rows below which a conversation mounts whole; defaults to one history batch. A strict bound passes 0. */
  readonly wholeMountSlack?: number
  /** Retained hidden panes pause progressive materialization until they become active again. */
  readonly active?: boolean
  /** Persistent controls share the materialization state without becoming part of the scroll content. */
  readonly renderHeader?: (startIndex: number, revealRow: (id: string, preservePosition?: boolean) => void, materializing: boolean) => React.ReactNode
  readonly children?: React.ReactNode | ((startIndex: number, revealRow: (id: string, preservePosition?: boolean) => void, materializing: boolean) => React.ReactNode)
}

export const historyBatchSize = 4

export const EmbraceScrollViewport = React.memo(function EmbraceScrollViewport({ items, anchorHistory, children, contentProps, stateKey, scrollToBottomKey, initialTailCount, wholeMountSlack = historyBatchSize, active = true, renderHeader, ...props }: EmbraceScrollViewportProps) {
  const store = React.useContext(ViewportStoreContext)
  const saved = stateKey === undefined ? undefined : store?.get(stateKey)
  const rememberedFirstId = saved?.following === false ? saved.firstRenderedId : undefined
  const [controller] = React.useState(() => new ViewportController(stateKey === undefined ? undefined : store?.get(stateKey)))
  // Do not defer a short conversation that fits within the tail plus the whole-mount slack.
  const initialStart = initialTailCount === undefined || items.length <= initialTailCount + wholeMountSlack ? 0 : items.length - initialTailCount
  const [window, setWindow] = React.useState(() => ({ key: stateKey, firstId: rememberedFirstId ?? items[initialStart]?.id }))
  const [revealTarget, setRevealTarget] = React.useState<{ readonly key: string | undefined; readonly id: string }>()
  const savedFirstId = window.key === stateKey ? window.firstId : rememberedFirstId
  const savedIndex = savedFirstId === undefined ? -1 : items.findIndex(item => item.id === savedFirstId)
  const startIndex = initialStart === 0 ? 0 : savedIndex >= 0 ? savedIndex : initialStart
  const firstId = items[startIndex]?.id
  const targetIndex = revealTarget?.key === stateKey ? items.findIndex(item => item.id === revealTarget?.id) : -1
  const materializing = targetIndex >= 0 && targetIndex < startIndex
  const revealRow = React.useCallback((id: string, preservePosition = false) => {
    const index = items.findIndex(item => item.id === id)
    if (index === -1) return
    if (!preservePosition) controller.reveal()
    else if (index < startIndex) controller.prepareMaterialize()
    setRevealTarget(index < startIndex ? { key: stateKey, id } : undefined)
  }, [controller, items, stateKey, startIndex])
  React.useLayoutEffect(() => {
    if (!active || !materializing) return
    const frame = requestAnimationFrame(() => {
      controller.prepareMaterialize()
      setWindow({ key: stateKey, firstId: items[Math.max(targetIndex, startIndex - historyBatchSize)]?.id })
    })
    return () => cancelAnimationFrame(frame)
  }, [active, controller, items, materializing, startIndex, stateKey, targetIndex])
  const previousItems = React.useRef(items)
  const previousKey = React.useRef(stateKey)
  const previousCommand = React.useRef(scrollToBottomKey)
  const history = React.useMemo<ViewportAnchorHistory>(() => anchorHistory ?? { _tag: 'Complete', includes: id => items.some(row => row.id === id) }, [anchorHistory, items])
  React.useLayoutEffect(() => {
    if (stateKey !== previousKey.current) {
      if (previousKey.current !== undefined) store?.save(previousKey.current, controller.released())
      previousKey.current = stateKey
      controller.resume(stateKey === undefined ? undefined : store?.get(stateKey))
    } else if (scrollToBottomKey !== undefined && scrollToBottomKey !== previousCommand.current) {
      // Following persists, so rows that commit after the command (the pending send) stay in view.
      controller.jump()
    } else if (previousItems.current !== items) {
      // Metadata-only snapshots still reach geometry; only new or changed rows count as unread.
      if (sameRows(previousItems.current, items)) controller.schedule()
      else controller.changed()
    }
    controller.updateHistory(history)
    // The first render is not a command, and a conversation switch adopts its key without scrolling.
    previousCommand.current = scrollToBottomKey
    previousItems.current = items
  }, [controller, items, store, stateKey, scrollToBottomKey, history])
  React.useLayoutEffect(() => {
    const windowChanged = controller.firstRenderedId !== firstId
    controller.firstRenderedId = firstId
    if (window.key !== stateKey || window.firstId === undefined && firstId !== undefined) setWindow({ key: stateKey, firstId })
    controller.loadEarlier = !active || materializing || startIndex === 0 ? undefined : () => setWindow({ key: stateKey, firstId: items[Math.max(0, startIndex - historyBatchSize)]?.id })
    if (windowChanged) controller.settle()
    controller.schedule()
    return () => { controller.loadEarlier = undefined }
  }, [active, controller, firstId, items, materializing, stateKey, startIndex, window.key, window.firstId])
  // Every layout commit can insert above a pressed row, including runtime adoption without new items.
  React.useLayoutEffect(() => { controller.preservePress() })
  // Mutation-phase saves precede the owning surface's layout effect that removes closed keys.
  React.useLayoutEffect(() => () => {
    if (previousKey.current !== undefined) store?.save(previousKey.current, controller.released())
  }, [controller, store])
  return <div {...stylex.props(styles.frame)}>
    {renderHeader?.(startIndex, revealRow, materializing)}
    <div {...props} style={{ ...props.style, overflowAnchor: 'none' }} ref={controller.attach}><div {...contentProps}>{typeof children === 'function' ? children(startIndex, revealRow, materializing) : children}</div></div>
    <Button ref={controller.attachJump} onPress={controller.jump} hidden {...stylex.props(styles.jump)}>New messages ↓</Button>
  </div>
})

const styles = stylex.create({
  frame: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minHeight: 0, minWidth: 0 },
  // A visible jump control gets its own dock, never covering a reader's current line.
  jump: { flexShrink: 0, marginInline: 'auto', marginBlock: spaceVars.md, paddingBlock: spaceVars.xs, paddingInline: spaceVars.md, borderRadius: radiusVars.full, borderWidth: spaceVars.hairline, borderStyle: 'solid', borderColor: borderVars.borderStrong, backgroundColor: surfaceVars.raised, color: textVars.fg, fontSize: typeVars.metaSize, cursor: 'pointer' },
})
