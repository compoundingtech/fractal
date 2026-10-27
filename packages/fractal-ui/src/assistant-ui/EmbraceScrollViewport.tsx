import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
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
export interface ViewportState { readonly top: number; readonly following: boolean; readonly unread: boolean; readonly firstRenderedId?: string }

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
    this.following = false
    this.pendingTop = undefined
    this.anchor = undefined
    if (this.frame !== undefined) cancelAnimationFrame(this.frame)
    this.frame = undefined
  }

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

  readonly released = (): ViewportState => ({ top: this.lastTop, following: this.following, unread: this.unread, firstRenderedId: this.firstRenderedId })

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

  /** Commit one window boundary before paint, using the existing following/anchor policy. */
  readonly settle = () => {
    const element = this.element
    if (element === null) return
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
      // Focusing the lane itself does not navigate to a row or change the reader's position.
      if (event.type === 'focusin' && event.target === element) return
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
  readonly contentProps?: React.HTMLAttributes<HTMLDivElement>
  /** Each key keeps its own scroll state across viewport mounts. */
  readonly stateKey?: string
  /** Host command: a changed, defined key resumes following and clears unread; an unchanged key never scrolls. */
  readonly scrollToBottomKey?: string
  /** Mount the tail first, then fill one extra viewport and prepend on upward navigation. */
  readonly initialTailCount?: number
  /** Retained hidden panes pause progressive materialization until they become active again. */
  readonly active?: boolean
  /** Persistent controls share the materialization state without becoming part of the scroll content. */
  readonly renderHeader?: (startIndex: number, revealRow: (id: string, preservePosition?: boolean) => void, materializing: boolean) => React.ReactNode
  readonly children?: React.ReactNode | ((startIndex: number, revealRow: (id: string, preservePosition?: boolean) => void, materializing: boolean) => React.ReactNode)
}

export const historyBatchSize = 4

export const EmbraceScrollViewport = React.memo(function EmbraceScrollViewport({ items, children, contentProps, stateKey, scrollToBottomKey, initialTailCount, active = true, renderHeader, ...props }: EmbraceScrollViewportProps) {
  const store = React.useContext(ViewportStoreContext)
  const saved = stateKey === undefined ? undefined : store?.get(stateKey)
  const rememberedFirstId = saved?.following === false ? saved.firstRenderedId : undefined
  const [controller] = React.useState(() => new ViewportController(stateKey === undefined ? undefined : store?.get(stateKey)))
  // Do not defer a short conversation that fits within the tail plus one history batch.
  const initialStart = initialTailCount === undefined || items.length <= initialTailCount + historyBatchSize ? 0 : items.length - initialTailCount
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
