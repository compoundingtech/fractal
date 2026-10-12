import type { ListLayout } from 'react-aria-components'
import { layoutKeyAtOffset, scrollToLayoutKey } from '../composition/virtual-layout'

/** The source transcript persists a mode or a stable entry, never a numeric bottom. */
export type ScrollAnchor =
  | { readonly _tag: 'Following' }
  | {
      readonly _tag: 'Reading'
      readonly entry: string | null
      readonly offset: number
      readonly scrollTop: number
    }

export type JumpResult =
  | { readonly _tag: 'Anchored' }
  | { readonly _tag: 'Missing' }
  | { readonly _tag: 'Superseded' }

type Row = { readonly id: string; readonly version?: string; readonly targetIds?: readonly string[] }
type ReadingPosition = { readonly key: string; readonly offset: number }

const FOLLOW_SLACK = 48
const USER_SCROLL_WINDOW_MS = 250

/** Write the real maximum, not an overshoot the browser clamps: saved and compared tops stay exact. */
const scrollToEnd = (element: HTMLElement) => {
  element.scrollTop = Math.max(0, element.scrollHeight - element.clientHeight)
}

/**
 * Follows the app's long-session virtual conversation. The kit has one renderer per
 * entry, so it needs no block/outline-to-entry mapping.
 * RAC measurement can move scrollTop without user input; only user scrolls leave follow mode.
 */
export class ScrollController {
  private element: HTMLDivElement | null = null
  private following: boolean
  private restoring: boolean
  private userInputAt = 0
  private userScrolling = false
  private olderNavigation = false
  private homeNavigation = false
  /** scrollTop when an older-navigation command began, until RAC moves the viewport. */
  private navigationTop: number | undefined
  /**
   * After older rows are inserted and before the anchor row mounts: the content
   * below the viewport top at insertion, then `step` once that estimate is applied.
   */
  private prependBottom: number | 'step' | undefined
  private anchor: ReadingPosition | undefined
  private pendingAnchor: ReadingPosition | undefined
  private restoreFrame: number | undefined
  private firstKey: string | undefined
  private rows: readonly Row[] = []
  private rowKeys = new Set<string>()
  private readonly rowIndex = new Map<string, number>()
  private readonly rowByTarget = new Map<string, string>()
  private geometry = { contentHeight: 0, viewportHeight: 0 }
  private pressed: ReadingPosition | undefined
  private pendingJump: { readonly key: string; readonly resolve: (result: JumpResult) => void } | undefined
  private leadingKey: string | undefined

  private finishJump(result: JumpResult) {
    const pending = this.pendingJump
    this.pendingJump = undefined
    pending?.resolve(result)
  }

  /** An older-navigation command owns the viewport before RAC scrolls/focuses. */
  readonly read = (home = false) => {
    this.following = false
    this.olderNavigation = true
    this.homeNavigation = home
    this.finishJump({ _tag: 'Superseded' })
    this.pendingAnchor = undefined
    this.restoring = false
    this.userInputAt = performance.now()
    this.userScrolling = true
    this.navigationTop = this.element?.scrollTop
    this.prependBottom = undefined
    if (this.restoreFrame !== undefined) cancelAnimationFrame(this.restoreFrame)
    this.restoreFrame = undefined
    this.captureAnchor()
  }

  readonly jumpTo = (key: string): Promise<JumpResult> => {
    this.finishJump({ _tag: 'Superseded' })
    if (this.restoreFrame !== undefined) cancelAnimationFrame(this.restoreFrame)
    this.restoreFrame = undefined
    this.pressed = undefined
    this.olderNavigation = false
    this.homeNavigation = false
    this.following = false
    this.navigationTop = undefined
    this.prependBottom = undefined
    this.restoring = false
    this.pendingAnchor = { key: this.rowByTarget.get(key) ?? key, offset: 0 }
    // The kit targets ES2022; this deferred completion needs its executor API.
    return new Promise<JumpResult>(resolve => {
      this.pendingJump = { key, resolve }
      this.restore()
    })
  }

  readonly position = (): ScrollAnchor => this.following
    ? { _tag: 'Following' }
    : { _tag: 'Reading', entry: this.anchor?.key ?? null, offset: this.anchor?.offset ?? 0, scrollTop: this.element?.scrollTop ?? 0 }

  constructor(
    private readonly options: {
      readonly layout: ListLayout<unknown>
      readonly initial: ScrollAnchor
      readonly save: (anchor: ScrollAnchor) => void
      readonly setUnread: (unread: boolean) => void
    },
  ) {
    this.following = options.initial._tag === 'Following'
    this.restoring = options.initial._tag === 'Reading'
    if (options.initial._tag === 'Reading' && options.initial.entry !== null) {
      this.pendingAnchor = { key: options.initial.entry, offset: options.initial.offset }
    }
  }

  readonly jump = () => {
    this.finishJump({ _tag: 'Superseded' })
    this.pressed = undefined
    this.following = true
    this.olderNavigation = false
    this.homeNavigation = false
    this.navigationTop = undefined
    this.prependBottom = undefined
    this.restoring = false
    this.pendingAnchor = undefined
    if (this.restoreFrame !== undefined) cancelAnimationFrame(this.restoreFrame)
    this.restoreFrame = undefined
    if (this.element !== null) scrollToEnd(this.element)
    this.options.setUnread(false)
    this.options.save({ _tag: 'Following' })
  }

  /** React 19 cleans up the ref when the actual RAC scroll element unmounts. */
  readonly attach = (element: HTMLDivElement | null) => {
    if (element === null) return
    this.element = element
    const onUserInput = () => {
      this.finishJump({ _tag: 'Superseded' })
      this.olderNavigation = false
      this.homeNavigation = false
      this.navigationTop = undefined
      this.prependBottom = undefined
      this.userInputAt = performance.now()
      this.userScrolling = true
      this.pendingAnchor = undefined
      this.restoring = false
      if (this.restoreFrame !== undefined) cancelAnimationFrame(this.restoreFrame)
      this.restoreFrame = undefined
    }
    const onScroll = () => {
      const resized =
        element.scrollHeight !== this.geometry.contentHeight ||
        element.clientHeight !== this.geometry.viewportHeight
      if (this.navigationTop !== undefined && element.scrollTop !== this.navigationTop) this.navigationTop = undefined
      if (this.restoring || this.pendingJump !== undefined || this.pressed !== undefined) return
      if (this.pendingAnchor?.key === this.leadingKey && this.pendingAnchor !== undefined) return
      if (this.pendingAnchor !== undefined && !this.userScrolling) return
      if (resized && this.following) return
      if (!this.userScrolling && performance.now() - this.userInputAt > USER_SCROLL_WINDOW_MS)
        return
      this.captureAnchor()
      if (this.homeNavigation && this.leadingKey !== undefined && this.anchor?.key === this.firstKey) {
        // RAC has reached the loaded head. Include its measured inline state
        // rather than retaining RAC's focus-ring offset above the first turn.
        this.anchor = { key: this.leadingKey, offset: 0 }
        this.pendingAnchor = this.anchor
        this.scheduleRestore()
        return
      }
      // Measurement may extend the bottom after a fling. Geometry is not a
      // request to leave follow mode, or to enter it while reading history.
      // An older-navigation command stays reading even if a trailing bottom
      // scroll arrives before RAC applies its key navigation. New input clears it.
      if (!resized && !this.olderNavigation) this.following =
        element.scrollHeight - element.scrollTop - element.clientHeight <= FOLLOW_SLACK
      if (this.following) this.options.setUnread(false)
      this.savePosition()
    }
    const onScrollEnd = () => {
      // Measurement can end an unmoved scroll after a key command but before RAC
      // applies it. That must not hand the viewport back to the pre-key anchor.
      if (this.navigationTop !== undefined) return
      if (this.userScrolling && !this.following && this.pendingAnchor === undefined &&
        this.pendingJump === undefined && this.pressed === undefined && !this.restoring) this.captureAnchor()
      this.userScrolling = false
    }
    let resizeFrame: number | undefined
    const geometryChanged = () => {
      // Correct visible geometry before paint; RAC's measured layout can settle
      // in the next frame, so retain the id/offset until the next reader input.
      // RAC owns the new navigation position until its scroll has arrived.
      // Freezing the pre-key row here would undo Home/PageUp as it mounts.
      if (this.userScrolling && !this.following && this.pressed === undefined &&
        this.pendingAnchor === undefined && !this.restoring) {
        this.geometry = { contentHeight: element.scrollHeight, viewportHeight: element.clientHeight }
        return
      }
      if (this.pressed !== undefined) this.pendingAnchor = this.pressed
      else if (!this.following && this.anchor !== undefined && this.pendingAnchor === undefined) this.pendingAnchor = this.anchor
      if (this.pendingAnchor !== undefined || this.restoring) this.restore()
      else if (this.following) scrollToEnd(element)
      if (resizeFrame !== undefined) return
      resizeFrame = requestAnimationFrame(() => {
        resizeFrame = undefined
        if (this.pendingAnchor !== undefined || this.restoring) this.restore()
        else if (this.following) scrollToEnd(element)
        this.geometry = { contentHeight: element.scrollHeight, viewportHeight: element.clientHeight }
      })
    }
    const observer = new ResizeObserver(geometryChanged)
    observer.observe(element)
    const observedRows = new Set<Element>()
    let sizer: Element | null = null
    const observeMountedGeometry = () => {
      const nextSizer = element.firstElementChild
      if (sizer !== nextSizer) {
        if (sizer !== null) observer.unobserve(sizer)
        sizer = nextSizer
        if (sizer !== null) observer.observe(sizer)
      }
      const mounted = new Set(element.querySelectorAll('[data-embrace-entry-id]'))
      for (const row of observedRows) {
        if (!mounted.has(row)) { observer.unobserve(row); observedRows.delete(row) }
      }
      for (const row of mounted) {
        if (!observedRows.has(row)) { observedRows.add(row); observer.observe(row) }
      }
    }
    // Measured entering rows can extend past RAC's current spacer without
    // resizing that spacer. Observe only mounted rows, including replacements.
    const mutations = new MutationObserver(() => {
      observeMountedGeometry()
      geometryChanged()
    })
    mutations.observe(element, { childList: true, subtree: true })
    observeMountedGeometry()
    const press = (event: PointerEvent) => {
      if (!(event.target instanceof Element)) return
      const row = event.target.closest<HTMLElement>('[data-embrace-entry-id]')
      const key = row?.dataset['embraceEntryId']
      if (row !== null && row !== undefined && key !== undefined && element.contains(row)) {
        this.pressed = { key, offset: row.getBoundingClientRect().top - element.getBoundingClientRect().top }
      }
    }
    const release = () => {
      if (this.pressed === undefined) return
      const held = this.pressed
      this.pressed = undefined
      // The held gesture is finished. Measurement-generated scroll events must
      // not replace its saved anchor before a new reader input takes ownership.
      this.userScrolling = false
      if (this.following) {
        this.pendingAnchor = undefined
        scrollToEnd(element)
      } else {
        this.anchor = held
        this.pendingAnchor = this.anchor
      }
    }
    element.addEventListener('pointerdown', press, true)
    element.ownerDocument.addEventListener('pointerup', release, true)
    element.ownerDocument.addEventListener('pointercancel', release, true)
    element.ownerDocument.defaultView?.addEventListener('blur', release)
    const inputs = ['wheel', 'touchmove', 'keydown', 'pointerdown'] as const
    for (const type of inputs) element.addEventListener(type, onUserInput, { passive: true })
    element.addEventListener('scroll', onScroll, { passive: true })
    element.addEventListener('scrollend', onScrollEnd, { passive: true })
    return () => {
      observer.disconnect()
      mutations.disconnect()
      if (resizeFrame !== undefined) cancelAnimationFrame(resizeFrame)
      if (this.restoreFrame !== undefined) cancelAnimationFrame(this.restoreFrame)
      this.restoreFrame = undefined
      for (const type of inputs) element.removeEventListener(type, onUserInput)
      element.removeEventListener('pointerdown', press, true)
      element.ownerDocument.removeEventListener('pointerup', release, true)
      element.ownerDocument.removeEventListener('pointercancel', release, true)
      element.ownerDocument.defaultView?.removeEventListener('blur', release)
      element.removeEventListener('scroll', onScroll)
      element.removeEventListener('scrollend', onScrollEnd)
      if (!this.restoring) this.savePosition()
      this.finishJump({ _tag: 'Superseded' })
      this.element = null
    }
  }

  private savePosition() {
    this.options.save(
      this.following
        ? { _tag: 'Following' }
        : {
            _tag: 'Reading',
            entry: this.anchor?.key ?? null,
            offset: this.anchor?.offset ?? 0,
            scrollTop: this.element?.scrollTop ?? 0,
          },
    )
  }

  private captureAnchor() {
    const element = this.element
    if (element === null) return
    const top = element.getBoundingClientRect().top
    let nearest: HTMLElement | undefined
    let nearestTop = Infinity
    for (const candidate of element.querySelectorAll<HTMLElement>('[data-embrace-entry-id]')) {
      const rect = candidate.getBoundingClientRect()
      if (rect.bottom > top && rect.top < top + element.clientHeight && rect.top < nearestTop) {
        nearest = candidate
        nearestTop = rect.top
      }
    }
    const key = nearest?.dataset['embraceEntryId']
    if (key !== undefined && this.rowKeys.has(key)) {
      this.anchor = { key, offset: nearestTop - top }
    } else {
      const leading = this.leadingKey === undefined ? null : this.options.layout.getLayoutInfo(this.leadingKey)
      const match = leading !== null && element.scrollTop < leading.rect.y + leading.rect.height
        ? { key: this.leadingKey!, info: leading }
        : layoutKeyAtOffset(this.options.layout, this.rows, element.scrollTop)
      if (match !== undefined) this.anchor = { key: String(match.key), offset: match.info.rect.y - element.scrollTop }
    }
    if (this.pendingAnchor !== undefined) this.pendingAnchor = this.anchor
  }

  private scheduleRestore() {
    if (this.restoreFrame !== undefined) return
    this.restoreFrame = requestAnimationFrame(() => {
      this.restoreFrame = undefined
      this.restore()
    })
  }

  private restore() {
    const anchor = this.pendingAnchor
    const element = this.element
    if (element === null) return
    const key = anchor !== undefined && this.rowKeys.has(anchor.key) ? anchor.key : undefined
    const info = key === undefined ? null : this.options.layout.getLayoutInfo(key)
    if (
      this.restoring &&
      key === undefined &&
      this.firstKey !== undefined &&
      this.options.layout.getLayoutInfo(this.firstKey) !== null &&
      element.scrollHeight > 0
    ) {
      // An absent saved entry can fall back only once collection geometry exists.
      element.scrollTop = this.options.initial._tag === 'Reading' ? this.options.initial.scrollTop : 0
      this.captureAnchor()
      this.pendingAnchor = this.anchor
      this.restoring = false
      return
    }
    const mounted = (target: string) => {
      for (const candidate of element.querySelectorAll<HTMLElement>(`[data-embrace-entry-id="${CSS.escape(target)}"]`)) {
        if (candidate.isConnected && candidate.getBoundingClientRect().height > 0) return candidate
      }
      return undefined
    }
    if (this.prependBottom !== undefined) {
      if (key === undefined) this.prependBottom = undefined
      else if (mounted(key) === undefined) {
        if (this.prependBottom !== 'step') {
          // RAC rebuilds only its requested rect: after an insert above the reader,
          // an unmounted anchor keeps its pre-insert layout y. Only content above
          // the reader changed, so the distance to the bottom is the best estimate.
          element.scrollTop = element.scrollHeight - this.prependBottom
          this.prependBottom = 'step'
          return
        }
        // Estimated sizes can still miss the row. Walk past the visible rows
        // toward it; each mutation re-enters here until the anchor row mounts.
        // Off-screen persisted rows (RAC keeps the focused one) do not count.
        const targetIndex = this.rowIndex.get(key)
        const viewport = element.getBoundingClientRect()
        let before: { index: number; bottom: number } | undefined
        let after: { index: number; top: number } | undefined
        for (const row of element.querySelectorAll<HTMLElement>('[data-embrace-entry-id]')) {
          const index = this.rowIndex.get(row.dataset['embraceEntryId'] ?? '')
          const rect = row.getBoundingClientRect()
          if (targetIndex === undefined || index === undefined || rect.height === 0 || rect.bottom <= viewport.top || rect.top >= viewport.bottom) continue
          if (index < targetIndex && (before === undefined || index > before.index)) before = { index, bottom: rect.bottom }
          if (index > targetIndex && (after === undefined || index < after.index)) after = { index, top: rect.top }
        }
        if (after === undefined && before !== undefined) { element.scrollTop += before.bottom - viewport.top; return }
        if (before === undefined && after !== undefined) { element.scrollTop -= viewport.bottom - after.top; return }
        // RAC has not rendered rows at the new position yet; its mutation re-enters.
        if (before === undefined && targetIndex !== undefined) return
        // Visible rows on both sides without the anchor, or the leading history
        // marker: fall back to the layout position.
        this.prependBottom = undefined
      } else this.prependBottom = undefined
    }
    // RAC publishes its collection after the parent commit. Keep the stable anchor
    // through estimated and measured layouts instead of consuming it prematurely.
    if (
      anchor !== undefined &&
      key !== undefined &&
      info !== null &&
      element.scrollHeight >= info.rect.y + Math.min(info.rect.height, element.clientHeight)
    ) {
      scrollToLayoutKey(this.options.layout, key, element, anchor.offset)
      const target = mounted(key)
      if (target !== undefined) {
        element.scrollTop +=
          target.getBoundingClientRect().top - element.getBoundingClientRect().top - anchor.offset
        this.anchor = { key, offset: anchor.offset }
        this.pendingAnchor = this.anchor
        this.restoring = false
        this.savePosition()
        if (this.pendingJump !== undefined && (this.rowByTarget.get(this.pendingJump.key) ?? this.pendingJump.key) === key) this.finishJump({ _tag: 'Anchored' })
      }
    }
    if (this.restoring || this.pendingJump !== undefined && this.rowKeys.has(this.rowByTarget.get(this.pendingJump.key) ?? this.pendingJump.key)) this.scheduleRestore()
  }

  /** Call after a commit that changes the generic entry collection. */
  afterRowsChange(rows: readonly Row[], leadingKey?: string, sourceIds?: ReadonlySet<string>) {
    const previous = this.rows
    const previousFirst = this.firstKey
    const previousLeading = this.leadingKey
    this.leadingKey = leadingKey
    this.rows = rows
    this.firstKey = rows[0]?.id
    if (previous !== rows || previousLeading !== leadingKey) {
      this.rowKeys.clear()
      this.rowIndex.clear()
      this.rowByTarget.clear()
      for (const [index, row] of rows.entries()) {
        this.rowKeys.add(row.id)
        this.rowIndex.set(row.id, index)
        for (const id of row.targetIds ?? []) this.rowByTarget.set(id, row.id)
      }
      if (leadingKey !== undefined) this.rowKeys.add(leadingKey)
    }
    if (leadingKey !== undefined && previousLeading !== leadingKey && !this.following &&
      (this.element?.scrollTop === 0 || this.homeNavigation && this.anchor?.key === this.firstKey)) {
      // The logical top includes the inline history state, not a focused turn
      // below it. Keep that edge stable while the collection adopts the marker.
      this.anchor = { key: leadingKey, offset: 0 }
      this.pendingAnchor = this.anchor
      this.scheduleRestore()
    }
    if (previousLeading !== undefined && previousLeading !== leadingKey && this.firstKey !== undefined) {
      // Removing a loading/error marker leaves the first loaded row at the top.
      if (this.anchor?.key === previousLeading) this.anchor = { key: this.firstKey, offset: 0 }
      if (this.pendingAnchor?.key === previousLeading) this.pendingAnchor = this.anchor
    }
    // News survives a held row action. Pinning affects geometry, not delivery.
    if (previous.length > 0 && previous !== rows && !this.restoring) {
      const start = rows.findIndex(row => row.id === previous[0]?.id)
      if (start < 0 || rows.length - start !== previous.length ||
        previous.some((row, index) => row.id !== rows[start + index]?.id ||
          (row.version === undefined ? row !== rows[start + index] : row.version !== rows[start + index]?.version))) {
        if (!this.following || this.pressed !== undefined) this.options.setUnread(true)
      }
    }
    if (this.restoring && this.element !== null && rows.length > 0) this.restore()
    if (this.pendingJump !== undefined) {
      const target = this.rowByTarget.get(this.pendingJump.key) ?? this.pendingJump.key
      if (!this.rowKeys.has(target)) {
        // Source presence and render readiness differ during runtime adoption.
        // A loaded item waits for its owning row; only absence means Missing.
        if (sourceIds?.has(this.pendingJump.key) !== true) {
          this.finishJump({ _tag: 'Missing' })
          this.pendingAnchor = undefined
        }
      } else {
        this.pendingAnchor = { key: target, offset: 0 }
        this.restore()
      }
      return
    }
    if (this.pressed !== undefined) {
      this.pendingAnchor = this.pressed
      this.restore()
      this.scheduleRestore()
      return
    }
    if (this.following) {
      this.options.setUnread(false)
      if (this.element !== null) scrollToEnd(this.element)
      return
    }
    if (
      this.anchor !== undefined &&
      previousFirst !== undefined &&
      this.firstKey !== previousFirst &&
      this.rowKeys.has(previousFirst)
    ) {
      // The collection rebuild follows this commit. ResizeObserver restores before
      // paint when the sizer grows; the tracked animation frame is the fallback.
      this.pendingAnchor = this.anchor
      // The sizer may already include the inserted rows; the settled geometry does not.
      if (this.element !== null) this.prependBottom = (this.geometry.contentHeight || this.element.scrollHeight) - this.element.scrollTop
      this.scheduleRestore()
    } else if (this.pendingAnchor !== undefined) {
      this.restore()
    }
  }
}

/**
 * Workshop-local storage; independent of the app's state registry. A missing, unreadable or
 * malformed value resumes following instead of throwing out of render.
 */
export function readScrollAnchor(key: string): ScrollAnchor {
  if (typeof window === 'undefined') return { _tag: 'Following' }
  let value: unknown
  try {
    const raw = window.localStorage.getItem(`fractal-ui.embrace:${key}:conversation.anchor`)
    if (raw === null) return { _tag: 'Following' }
    value = JSON.parse(raw)
  } catch {
    return { _tag: 'Following' }
  }
  if (
    typeof value === 'object' && value !== null && '_tag' in value &&
    value._tag === 'Reading' &&
    'entry' in value && (value.entry === null || typeof value.entry === 'string') &&
    'offset' in value && typeof value.offset === 'number' && Number.isFinite(value.offset) &&
    'scrollTop' in value && typeof value.scrollTop === 'number' && Number.isFinite(value.scrollTop)
  ) return { _tag: 'Reading', entry: value.entry, offset: value.offset, scrollTop: value.scrollTop }
  return { _tag: 'Following' }
}

export function saveScrollAnchor(key: string, anchor: ScrollAnchor) {
  if (typeof window !== 'undefined') {
    window.localStorage.setItem(`fractal-ui.embrace:${key}:conversation.anchor`, JSON.stringify(anchor))
  }
}
