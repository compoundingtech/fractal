import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, ButtonContext, GridList, GridListItem, Virtualizer } from 'react-aria-components'
import { ScrollController, type JumpResult, type ScrollAnchor } from './embrace-virtual/ScrollController'
import { AnchoredListLayout, measuredListLayoutOptions } from './composition/virtual-layout'
import { RuntimeVisibleIds } from './EmbraceRuntime'
import { surfaceVars, textVars, borderVars, radiusVars, spaceVars, typeVars, geometryNumbers } from './composition-tokens.stylex'

const rowSelector = '[data-item-id], [data-embrace-entry-id]'
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
export interface ViewportState { readonly top: number; readonly following: boolean; readonly unread: boolean; readonly anchor?: ScrollAnchor }

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

/** Scroll and row geometry are imperative; they never invalidate the message subtree. */
class ViewportController {
  private element: HTMLDivElement | null = null
  private jumpButton: HTMLButtonElement | null = null
  private following = true
  private anchor: { element: HTMLElement; offset: number } | undefined
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

  constructor(saved?: ViewportState) {
    if (saved !== undefined && !saved.following) {
      this.restored = saved
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

  readonly released = (): ViewportState => ({ top: this.lastTop, following: this.following, unread: this.unread })

  /** Swaps a reused viewport to another conversation without carrying its unread mark across. */
  readonly resume = (saved?: ViewportState) => {
    this.unread = saved !== undefined && !saved.following && saved.unread
    this.dock()
    if (saved !== undefined && !saved.following) {
      this.following = false
      this.anchor = undefined
      this.pendingTop = saved.top
      this.writeTop(saved.top)
      this.scheduleCapture()
      this.schedule()
    } else {
      this.following = true
      this.anchor = undefined
      if (this.element !== null) this.writeTop(this.element.scrollHeight)
      this.schedule()
    }
  }

  private capture() {
    const element = this.element
    if (element === null) return
    const viewport = element.getBoundingClientRect()
    const top = viewport.top
    const hit = element.ownerDocument.elementFromPoint(viewport.left + viewport.width / 2, top + geometryNumbers.scrollEndTolerance)?.closest<HTMLElement>(rowSelector)
    if (hit !== undefined && hit !== null && element.contains(hit)) {
      this.anchor = { element: hit, offset: hit.getBoundingClientRect().top - top }
      return
    }
    for (const row of element.querySelectorAll<HTMLElement>(rowSelector)) {
      const bounds = row.getBoundingClientRect()
      if (bounds.bottom > top) {
        this.anchor = { element: row, offset: bounds.top - top }
        return
      }
    }
  }
  private scheduleCapture() {
    if (this.captureFrame !== undefined) return
    this.captureFrame = requestAnimationFrame(() => {
      this.captureFrame = undefined
      if (!this.following) this.capture()
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
    if (this.anchor?.element.isConnected) this.anchor.offset = this.anchor.element.getBoundingClientRect().top - viewport.getBoundingClientRect().top
    return true
  }

  readonly schedule = () => {
    if (this.frame !== undefined) return
    this.frame = requestAnimationFrame(() => {
      this.frame = undefined
      const element = this.element
      if (element === null) return
      // The pressed row takes precedence over the reader's history anchor.
      if (this.preservePress()) { this.lastWidth = element.clientWidth; return }
      if (this.following) this.writeTop(element.scrollHeight)
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
    })
  }

  readonly jump = () => {
    this.following = true
    this.unread = false
    this.anchor = undefined
    this.pendingTop = undefined
    this.dock()
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
      if (event instanceof KeyboardEvent && (navigationKeys[event.key] !== true || (event.target instanceof HTMLElement && event.target.closest('input,textarea,[contenteditable="true"]')))) return
      if (event.type === 'wheel' || event.type === 'touchmove' || event.type === 'keydown') this.pressedAnchor = undefined
      this.following = false
      this.programmaticTop = undefined
      this.pendingTop = undefined
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
      // A real reader scroll (including scrollbar dragging) takes ownership until the next press.
      this.pressedAnchor = undefined
      this.lastTop = element.scrollTop
      this.following = element.scrollHeight - element.clientHeight - element.scrollTop <= geometryNumbers.scrollEndTolerance
      if (this.following) {
        this.unread = false
        this.dock()
        this.anchor = undefined
      } else this.scheduleCapture()
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
export interface ViewportRow { readonly id: string; readonly version?: string; readonly targetIds?: readonly string[] }

/** A republished snapshot with the same rows is not news: compare ids and content versions, not array identity. */
function sameRows(previous: readonly ViewportRow[], next: readonly ViewportRow[]) {
  if (previous.length !== next.length) return false
  return next.every((row, index) => {
    const before = previous[index]!
    return before.id === row.id && (row.version === undefined ? before === row : before.version === row.version)
  })
}

export interface EmbraceScrollViewportProps extends React.HTMLAttributes<HTMLDivElement> {
  readonly items: readonly ViewportRow[]
  readonly contentProps?: React.HTMLAttributes<HTMLDivElement>
  /** Each key keeps its own scroll state across viewport mounts. */
  readonly stateKey?: string
  /** Host command: a changed, defined key resumes following and clears unread; an unchanged key never scrolls. */
  readonly scrollToBottomKey?: string
}

export const EmbraceScrollViewport = React.memo(function EmbraceScrollViewport({ items, children, contentProps, stateKey, scrollToBottomKey, ...props }: EmbraceScrollViewportProps) {
  const store = React.useContext(ViewportStoreContext)
  const [controller] = React.useState(() => new ViewportController(stateKey === undefined ? undefined : store?.get(stateKey)))
  const previousItems = React.useRef(items)
  const previousKey = React.useRef(stateKey)
  const previousCommand = React.useRef(scrollToBottomKey)
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
    // The first render is not a command, and a conversation switch adopts its key without scrolling.
    previousCommand.current = scrollToBottomKey
    previousItems.current = items
  }, [controller, items, store, stateKey, scrollToBottomKey])
  // Every layout commit can insert above a pressed row, including runtime adoption without new items.
  React.useLayoutEffect(() => { controller.preservePress() })
  // Mutation-phase saves precede the owning surface's layout effect that removes closed keys.
  React.useLayoutEffect(() => () => {
    if (previousKey.current !== undefined) store?.save(previousKey.current, controller.released())
  }, [controller, store])
  return <div {...stylex.props(styles.frame)}>
    <div {...props} style={{ ...props.style, overflowAnchor: 'none' }} ref={controller.attach}><div {...contentProps}>{children}</div></div>
    <Button ref={controller.attachJump} onPress={controller.jump} hidden {...stylex.props(styles.jump)}>New messages ↓</Button>
  </div>
})

export type { JumpResult } from './embrace-virtual/ScrollController'
export interface VirtualViewportHandle { readonly jumpTo: (id: string) => Promise<JumpResult> }
/**
 * What the host knows about rows older than the loaded head. `Pending` is unknown (older rows may
 * exist, the host cannot page yet); `More` pages through `loadOlder`; `Loading`/`Failed` report a
 * request in flight or rejected; `Complete` means the loaded head is the start of the conversation.
 */
export type VirtualViewportHistory =
  | { readonly _tag: 'Pending' }
  | { readonly _tag: 'Complete' }
  | { readonly _tag: 'More'; readonly loadOlder: () => Promise<void> }
  | { readonly _tag: 'Loading' }
  | { readonly _tag: 'Failed'; readonly retry: () => Promise<void>; readonly detail?: string }

export interface EmbraceVirtualScrollViewportProps<T extends ViewportRow> extends Omit<EmbraceScrollViewportProps, 'items' | 'children'> {
  /** Rows keyed by stable id, oldest first. Server output renders all of them; client lanes virtualize them. */
  readonly items: readonly T[]
  /** Every loaded row id and source alias, including rows awaiting adoption; ids outside it settle `Missing`. */
  readonly sourceIds?: ReadonlySet<string>
  readonly renderItem: (item: T) => React.ReactNode
  readonly history: VirtualViewportHistory
  /** The leading history row. `request` runs `More.loadOlder` or `Failed.retry`, one request at a time. */
  readonly boundary: (request: () => void) => React.ReactNode
  /** Shown in the leading row when there are no items. */
  readonly empty: React.ReactNode
  readonly ref?: React.Ref<VirtualViewportHandle>
}

const historyKey = '__fractal_history__'
const noSubscription = () => () => undefined

/**
 * RAC owns the bounded collection; the controller owns id-keyed measured anchoring. The leading
 * history row is an explicit slot ahead of the rows. Before measurement (server output and the
 * hydration pass that adopts it) the boundary and every row render in a static lane, so nothing
 * waits on client geometry; afterwards the same ids become collection rows, each rendered once.
 * A row keeps its id, and so its DOM, through state changes such as a pending send's acknowledgement.
 */
export function EmbraceVirtualScrollViewport<T extends ViewportRow>({
  items, sourceIds, renderItem, history, boundary, empty, stateKey, scrollToBottomKey, ref, contentProps, ...props
}: EmbraceVirtualScrollViewportProps<T>) {
  // False for server output and the hydration pass that adopts it; true for every client render after.
  const measurable = React.useSyncExternalStore(noSubscription, () => true, () => false)
  // RAC's hidden collection creates React resources even for unmounted rows.
  // Frame-sized metadata adoption bounds commits without narrowing source ids.
  const [collectionCount, setCollectionCount] = React.useState(64)
  React.useEffect(() => {
    if (!measurable || collectionCount >= items.length) return
    const frame = requestAnimationFrame(() => setCollectionCount(count => Math.min(items.length, count + 64)))
    return () => cancelAnimationFrame(frame)
  }, [measurable, collectionCount, items.length])
  const collectionReady = React.useRef(false)
  collectionReady.current = collectionCount >= items.length
  const publishVisible = React.useContext(RuntimeVisibleIds)
  const demandOwner = React.useRef(Symbol('transcript viewport'))
  React.useLayoutEffect(() => () => publishVisible?.(demandOwner.current, undefined), [publishVisible])
  const rowsById = React.useMemo(() => new Map(items.map(item => [item.id, item])), [items])
  const rowsRef = React.useRef(rowsById)
  rowsRef.current = rowsById
  const store = React.useContext(ViewportStoreContext)
  const state = stateKey === undefined ? undefined : store?.get(stateKey)
  const [layout] = React.useState(() => new AnchoredListLayout())
  const [unread, setUnread] = React.useState(state?.unread ?? false)
  const unreadRef = React.useRef(unread)
  unreadRef.current = unread
  const [controller] = React.useState(() => new ScrollController({
    layout,
    initial: state?.anchor ?? (state !== undefined && !state.following
      ? { _tag: 'Reading', entry: null, offset: 0, scrollTop: state.top }
      : { _tag: 'Following' }),
    setUnread: value => { unreadRef.current = value; setUnread(value) },
    save: anchor => {
      if (stateKey !== undefined) store?.save(stateKey, {
        anchor, top: anchor._tag === 'Reading' ? anchor.scrollTop : 0,
        following: anchor._tag === 'Following', unread: unreadRef.current,
      })
    },
  }))
  const element = React.useRef<HTMLDivElement | null>(null)
  const current = React.useRef(history)
  current.current = history
  const flight = React.useRef<Promise<void> | undefined>(undefined)
  // One history request at a time; the host reports its outcome through the next `history`.
  const invokeHistory = React.useCallback((action: () => Promise<void>): Promise<void> => {
    if (flight.current !== undefined) return flight.current
    controller.prepareForPrepend()
    const promise = Promise.resolve().then(action).catch(error => {
      console.warn('Transcript history request rejected; waiting for host history state.', error)
    }).finally(() => {
      if (flight.current === promise) flight.current = undefined
    })
    flight.current = promise
    return promise
  }, [controller])
  const request = React.useCallback(() => {
    const source = current.current
    if (source._tag === 'More') void invokeHistory(source.loadOlder)
    else if (source._tag === 'Failed') void invokeHistory(source.retry)
  }, [invokeHistory])
  // Older pages follow reader intent only: an upward gesture arms one request, which fires once the
  // lane is within the boundary. Mounting, following, restores and reflow never fetch history.
  const armed = React.useRef(false)
  const prefetch = React.useCallback(() => {
    const viewport = element.current
    const source = current.current
    if (viewport === null || !armed.current || !collectionReady.current || source._tag !== 'More') return
    if (viewport.scrollTop > viewport.clientHeight * 1.5) return
    armed.current = false
    void invokeHistory(source.loadOlder)
  }, [invokeHistory])
  React.useEffect(() => { if (collectionReady.current) prefetch() }, [collectionCount, prefetch])
  const attach = React.useCallback((viewport: HTMLDivElement | null) => {
    element.current = viewport
    if (viewport === null) return
    const cleanup = controller.attach(viewport)
    // Intent is independent of scroll events: at the top edge there may be
    // nothing left to scroll, but the reader can still request the next page.
    const olderGesture = (home = false) => {
      controller.read(home)
      armed.current = true
      prefetch()
    }
    let draggingScrollbar = false
    let pointerY = 0
    let dragRequested = false
    let touchY: number | undefined
    let touchRequested = false
    const onPointerDown = (event: PointerEvent) => {
      draggingScrollbar = event.target === viewport && event.pointerType !== 'touch'
      pointerY = event.clientY
      dragRequested = false
    }
    const onPointerMove = (event: PointerEvent) => {
      if (draggingScrollbar && !dragRequested && event.clientY < pointerY) {
        dragRequested = true
        olderGesture()
      }
      pointerY = event.clientY
    }
    const onPointerUp = () => { draggingScrollbar = false }
    const onTouchStart = (event: TouchEvent) => {
      touchY = event.touches[0]?.clientY
      touchRequested = false
    }
    const onTouchMove = (event: TouchEvent) => {
      const y = event.touches[0]?.clientY
      if (y !== undefined && touchY !== undefined && y > touchY && !touchRequested) {
        touchRequested = true
        olderGesture()
      }
      touchY = y
    }
    const onTouchEnd = () => { touchY = undefined }
    // A scroll reaching the boundary completes an armed gesture; it never arms one.
    const onScroll = () => prefetch()
    const onWheel = (event: WheelEvent) => { if (event.deltaY < 0) olderGesture() }
    const onKey = (event: KeyboardEvent) => {
      if (!(event.target instanceof HTMLElement) || event.target.closest('input,textarea,[contenteditable="true"]')) return
      if (event.key === 'PageUp' || event.key === 'Home' || event.key === 'ArrowUp') olderGesture(event.key === 'Home')
    }
    const page = viewport.ownerDocument
    viewport.addEventListener('scroll', onScroll, { passive: true })
    viewport.addEventListener('touchstart', onTouchStart, { passive: true })
    viewport.addEventListener('touchmove', onTouchMove, { passive: true })
    viewport.addEventListener('touchend', onTouchEnd, { passive: true })
    viewport.addEventListener('touchcancel', onTouchEnd, { passive: true })
    viewport.addEventListener('wheel', onWheel, { passive: true })
    viewport.addEventListener('keydown', onKey)
    viewport.addEventListener('pointerdown', onPointerDown)
    page.addEventListener('pointermove', onPointerMove, { passive: true })
    page.addEventListener('pointerup', onPointerUp)
    page.addEventListener('pointercancel', onPointerUp)
    return () => {
      cleanup?.()
      viewport.removeEventListener('scroll', onScroll)
      viewport.removeEventListener('touchstart', onTouchStart)
      viewport.removeEventListener('touchmove', onTouchMove)
      viewport.removeEventListener('touchend', onTouchEnd)
      viewport.removeEventListener('touchcancel', onTouchEnd)
      viewport.removeEventListener('wheel', onWheel)
      viewport.removeEventListener('keydown', onKey)
      viewport.removeEventListener('pointerdown', onPointerDown)
      page.removeEventListener('pointermove', onPointerMove)
      page.removeEventListener('pointerup', onPointerUp)
      page.removeEventListener('pointercancel', onPointerUp)
      element.current = null
    }
  }, [controller, prefetch])
  React.useLayoutEffect(() => {
    const node = element.current
    if (!measurable || node === null || publishVisible === undefined) return
    const publish = () => {
      // Loading can begin before the first visible row has adopted. Capture its
      // measured position once it mounts, still before the older page arrives.
      if (current.current._tag === 'Loading') controller.prepareForPrepend()
      const ids: string[] = []
      for (const row of node.querySelectorAll<HTMLElement>('[data-embrace-entry-id]')) {
        const item = rowsRef.current.get(row.dataset['embraceEntryId'] ?? '')
        if (item !== undefined) ids.push(...(item.targetIds ?? [item.id]))
      }
      publishVisible(demandOwner.current, ids)
    }
    const observer = new MutationObserver(publish)
    observer.observe(node, { childList: true, subtree: true })
    publish()
    return () => observer.disconnect()
  }, [measurable, publishVisible, rowsById, controller])
  const command = React.useRef(scrollToBottomKey)
  // A jump needs a source-command commit even when the loaded snapshot is unchanged.
  const [jumpCommit, requestJumpCommit] = React.useReducer((count: number) => count + 1, 0)
  React.useLayoutEffect(() => {
    if (!measurable) return
    if (scrollToBottomKey !== undefined && command.current !== scrollToBottomKey) controller.jump()
    command.current = scrollToBottomKey
    controller.afterRowsChange(items, historyKey, sourceIds)
  }, [measurable, controller, items, sourceIds, scrollToBottomKey, jumpCommit])
  React.useImperativeHandle(ref, () => ({
    jumpTo: (id: string) => {
      const result = controller.jumpTo(id)
      requestJumpCommit()
      return result
    },
  }), [controller])
  const emptyContent = items.length === 0 ? empty : null
  const leading = React.useMemo(() => <ButtonContext.Provider value={null}>{boundary(request)}{emptyContent}</ButtonContext.Provider>, [boundary, request, emptyContent])
  const [collectionRows] = React.useState(() => new WeakMap<T, { readonly id: string; readonly item: T; readonly content: undefined }>())
  const collection = React.useMemo(() => [
    { id: historyKey, item: undefined, content: leading },
    ...items.slice(-collectionCount).map(item => {
      const cached = collectionRows.get(item)
      if (cached !== undefined) return cached
      const row = { id: item.id, item, content: undefined }
      collectionRows.set(item, row)
      return row
    }),
  ], [items, leading, collectionRows, collectionCount])
  if (!measurable) return <div {...stylex.props(styles.frame)}>
    <div {...props} aria-busy={history._tag === 'Loading'} style={{ ...props.style, overflowAnchor: 'none' }}>
      <div data-embrace-entry-id={historyKey} {...contentProps}>{leading}</div>
      {items.map(item => <div key={item.id} data-embrace-entry-id={item.id} {...contentProps}><ButtonContext.Provider value={null}>{renderItem(item)}</ButtonContext.Provider></div>)}
    </div>
  </div>
  return <div {...stylex.props(styles.frame)}>
    <Virtualizer layout={layout} layoutOptions={measuredListLayoutOptions} shouldObserveItemSize>
      <GridList {...props} ref={attach} items={collection} dependencies={[renderItem, contentProps]} keyboardNavigationBehavior="tab" render={domProps => <div {...domProps} data-embrace-collection-ready={collectionCount >= items.length} aria-busy={history._tag === 'Loading'} />} style={{ ...props.style, overflowAnchor: 'none' }}>
        {row => <GridListItem id={row.id} textValue={row.id} {...stylex.props(styles.virtualRow)}>
          <div data-embrace-entry-id={row.id} {...contentProps}>{row.item === undefined ? row.content : <ButtonContext.Provider value={null}>{renderItem(row.item)}</ButtonContext.Provider>}</div>
        </GridListItem>}
      </GridList>
    </Virtualizer>
    <Button onPress={controller.jump} hidden={!unread} {...stylex.props(styles.jump)}>New messages ↓</Button>
  </div>
}

const styles = stylex.create({
  frame: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minHeight: 0, minWidth: 0 },
  // RAC's ScrollView sets `pointer-events: none` on its content for 300 ms after
  // every scroll event, including the controller's own follow and restore writes.
  // A following lane scrolls on each streamed growth, so rows would refuse clicks
  // for as long as a reply streams. Rows opt back in; the lane stays scrollable.
  virtualRow: { outlineStyle: 'none', display: 'flow-root', minWidth: 0, pointerEvents: 'auto' },
  // A visible jump control gets its own dock, never covering a reader's current line.
  jump: { flexShrink: 0, marginInline: 'auto', marginBlock: spaceVars.md, paddingBlock: spaceVars.xs, paddingInline: spaceVars.md, borderRadius: radiusVars.full, borderWidth: spaceVars.hairline, borderStyle: 'solid', borderColor: borderVars.borderStrong, backgroundColor: surfaceVars.raised, color: textVars.fg, fontSize: typeVars.metaSize, cursor: 'pointer' },
})
