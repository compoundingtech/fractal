import type { ListLayout } from 'react-aria-components'
import { layoutKeyAtOffset, scrollToLayoutKey } from '../composition/virtual-layout'

/**
 * A source leaf inside an entry: a message, a tool row, or a work summary that owns folded
 * members. When an entry is split or renamed, the captured member names the reader's current entry.
 */
export type ScrollMember = { readonly id: string; readonly offset: number; readonly summary: boolean }

/** The source transcript persists a mode or a stable entry, never a numeric bottom. */
export type ScrollAnchor =
  | { readonly _tag: 'Following' }
  | {
      readonly _tag: 'Reading'
      readonly entry: string | null
      readonly offset: number
      readonly scrollTop: number
      readonly member?: ScrollMember
    }

export type JumpResult =
  | { readonly _tag: 'Anchored' }
  | { readonly _tag: 'Missing' }
  | { readonly _tag: 'Superseded' }

type Row = { readonly id: string; readonly version?: string; readonly targetIds?: readonly string[] }
type ReadingPosition = { readonly key: string; readonly offset: number; readonly member?: ScrollMember }

/** Anchorable leaves: work summaries list their source members; other leaves are one source item. */
const leafSelector = '[data-scroll-anchor-id], [data-item-id]'
const summaryMembers = (leaf: HTMLElement): readonly string[] => {
  try {
    const members: unknown = JSON.parse(leaf.dataset['scrollAnchorMembers'] ?? '[]')
    return Array.isArray(members) ? members.filter((member): member is string => typeof member === 'string') : []
  } catch {
    return []
  }
}

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
  /** The latest reader input has already moved the viewport away from `inputTop`. */
  private inputScrolled = false
  private inputTop: number | undefined
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
  /** Full loaded source membership, including items whose owning entry is not committed yet. */
  private sourceIds: ReadonlySet<string> | undefined
  private geometry = { contentHeight: 0, viewportHeight: 0 }
  private pressed: ReadingPosition | undefined
  private pressedPointer: number | undefined
  /** Pointers down anywhere on the page. */
  private readonly pointers = new Set<number>()
  /**
   * A press that ended following holds the jump dock until every pointer releases:
   * the reader did not ask for news yet. A reader already in history sees news at once.
   */
  private dockHeld = false
  /** The unread mark requested while the dock was held. */
  private deferredUnread: boolean | undefined
  private pendingJump: { readonly key: string; readonly resolve: (result: JumpResult) => void } | undefined
  private leadingKey: string | undefined

  private finishJump(result: JumpResult) {
    const pending = this.pendingJump
    this.pendingJump = undefined
    pending?.resolve(result)
  }

  private setUnread(unread: boolean) {
    if (this.dockHeld) this.deferredUnread = unread
    else this.options.setUnread(unread)
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
    this.inputScrolled = false
    this.inputTop = this.element?.scrollTop
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
    : { _tag: 'Reading', entry: this.anchor?.key ?? null, offset: this.anchor?.offset ?? 0, scrollTop: this.element?.scrollTop ?? 0, member: this.anchor?.member }

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
      this.pendingAnchor = { key: options.initial.entry, offset: options.initial.offset, member: options.initial.member }
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
    this.setUnread(false)
    this.options.save({ _tag: 'Following' })
  }

  /** React 19 cleans up the ref when the actual RAC scroll element unmounts. */
  readonly attach = (element: HTMLDivElement | null) => {
    if (element === null) return
    this.element = element
    const onUserInput = (event: Event) => {
      // A reader scroll or key takes ownership from a held row; a press does not.
      if (event.type !== 'pointerdown') this.pressed = undefined
      this.finishJump({ _tag: 'Superseded' })
      this.olderNavigation = false
      this.homeNavigation = false
      this.navigationTop = undefined
      this.prependBottom = undefined
      this.userInputAt = performance.now()
      this.userScrolling = true
      this.inputScrolled = false
      this.inputTop = element.scrollTop
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
      // Once the reader's scroll has moved and ended, a scroll that moves with the geometry
      // is the layout's: RAC clamps scrollTop to an estimated content size as rows change.
      // A key command whose scroll has not arrived yet still owns the next move.
      if (!this.userScrolling && (resized && this.inputScrolled || performance.now() - this.userInputAt > USER_SCROLL_WINDOW_MS))
        return
      // An unmoved scroll event (RAC re-asserting the focused row) is not the input's scroll.
      if (element.scrollTop !== this.inputTop) this.inputScrolled = true
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
      if (this.following) this.setUnread(false)
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
    // Pointers are tracked document-wide: a second pointer anywhere keeps a held dock still.
    const page = element.ownerDocument
    const view = page.defaultView
    const press = (event: PointerEvent) => {
      this.pointers.add(event.pointerId)
      if (!(event.target instanceof Element) || !element.contains(event.target)) return
      // A press in the lane engages the reader: rows arriving meanwhile wait behind the jump.
      if (this.following) this.dockHeld = true
      this.following = false
      if (this.pressed !== undefined) return
      const row = event.target.closest<HTMLElement>('[data-embrace-entry-id]')
      const key = row?.dataset['embraceEntryId']
      if (row !== null && row !== undefined && key !== undefined) {
        this.pressed = { key, offset: row.getBoundingClientRect().top - element.getBoundingClientRect().top }
        this.pressedPointer = event.pointerId
      }
    }
    const releasePress = () => {
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
    const dock = () => {
      if (this.pointers.size > 0 || !this.dockHeld) return
      this.dockHeld = false
      const unread = this.deferredUnread
      this.deferredUnread = undefined
      if (unread !== undefined) this.options.setUnread(unread)
    }
    const release = (event: PointerEvent) => {
      this.pointers.delete(event.pointerId)
      if (event.pointerId === this.pressedPointer) releasePress()
      dock()
    }
    // A release the page never sees (the window blurs, the tab hides) must not latch the press or the dock.
    const abandon = () => {
      this.pointers.clear()
      releasePress()
      dock()
    }
    const hidden = () => { if (page.visibilityState === 'hidden') abandon() }
    page.addEventListener('pointerdown', press, true)
    page.addEventListener('pointerup', release, true)
    page.addEventListener('pointercancel', release, true)
    page.addEventListener('lostpointercapture', release, true)
    page.addEventListener('visibilitychange', hidden)
    view?.addEventListener('blur', abandon)
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
      page.removeEventListener('pointerdown', press, true)
      page.removeEventListener('pointerup', release, true)
      page.removeEventListener('pointercancel', release, true)
      page.removeEventListener('lostpointercapture', release, true)
      page.removeEventListener('visibilitychange', hidden)
      view?.removeEventListener('blur', abandon)
      // Detached mid-press: the saved state keeps the mark the press deferred.
      this.pointers.clear()
      dock()
      element.removeEventListener('scroll', onScroll)
      element.removeEventListener('scrollend', onScrollEnd)
      if (!this.restoring) this.savePosition()
      this.finishJump({ _tag: 'Superseded' })
      this.element = null
    }
  }

  private savePosition() {
    this.options.save(this.position())
  }

  /** The first owned source leaf crossing the viewport top; a summary keeps a member it still owns. */
  private memberAt(row: HTMLElement, key: string, top: number): ScrollMember | undefined {
    for (const leaf of row.querySelectorAll<HTMLElement>(leafSelector)) {
      if (leaf.querySelector(leafSelector) !== null) continue
      const rect = leaf.getBoundingClientRect()
      if (rect.height === 0 || rect.bottom <= top) continue
      if (leaf.dataset['scrollAnchorId'] !== undefined) {
        const members = summaryMembers(leaf)
        const previous = this.anchor?.member
        const id = previous?.summary === true && members.includes(previous.id) ? previous.id : members[0]
        if (id !== undefined) return { id, offset: rect.top - top, summary: true }
      } else {
        const id = leaf.dataset['itemId']!
        if (this.rowByTarget.get(id) === key) return { id, offset: rect.top - top, summary: false }
      }
    }
    return undefined
  }

  /** The member's current leaf; a tool row folded after its entry remounted is represented by its summary. */
  private memberLeaf(row: HTMLElement, member: ScrollMember): HTMLElement | undefined {
    let summary: HTMLElement | undefined
    for (const leaf of row.querySelectorAll<HTMLElement>(leafSelector)) {
      if (leaf.dataset['scrollAnchorId'] !== undefined) {
        if (summary === undefined && summaryMembers(leaf).includes(member.id)) {
          if (member.summary) return leaf
          summary = leaf
        }
      } else if (!member.summary && leaf.dataset['itemId'] === member.id && leaf.querySelector(leafSelector) === null) return leaf
    }
    return summary
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
    if (nearest !== undefined && key !== undefined && this.rowKeys.has(key)) {
      this.anchor = { key, offset: nearestTop - top, member: this.memberAt(nearest, key, top) }
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
    // A split or backfilled entry can be renamed, or its captured member can move to another
    // entry while the old id survives: the member's current owner names the reader's row.
    const owner = anchor?.member === undefined ? undefined : this.rowByTarget.get(anchor.member.id)
    // A loaded member whose entry is not committed yet (runtime adoption) is not absent.
    if (anchor?.member !== undefined && owner === undefined && this.sourceIds?.has(anchor.member.id) === true) return
    const key = anchor === undefined ? undefined : owner ?? (this.rowKeys.has(anchor.key) ? anchor.key : undefined)
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
      // Compare ids directly: no selector escaping, so any source id is safe.
      for (const candidate of element.querySelectorAll<HTMLElement>('[data-embrace-entry-id]')) {
        if (candidate.dataset['embraceEntryId'] === target && candidate.isConnected && candidate.getBoundingClientRect().height > 0) return candidate
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
        // The old entry offset does not locate a member that moved: its leaf is the reader's line.
        const leaf = key === anchor.key || anchor.member === undefined ? undefined : this.memberLeaf(target, anchor.member)
        if (leaf !== undefined) element.scrollTop += leaf.getBoundingClientRect().top - element.getBoundingClientRect().top - anchor.member!.offset
        this.anchor = { key, offset: leaf === undefined ? anchor.offset : target.getBoundingClientRect().top - element.getBoundingClientRect().top, member: anchor.member }
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
    this.sourceIds = sourceIds
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
    if (leadingKey !== undefined && previousLeading !== leadingKey && !this.following && !this.restoring &&
      (this.element?.scrollTop === 0 || this.homeNavigation && this.anchor?.key === this.firstKey)) {
      // The logical top includes the inline history state, not a focused turn
      // below it. Keep that edge stable while the collection adopts the marker.
      // A mount restoring a saved reading position owns its anchor: its
      // unrestored scrollTop of 0 is not the reader's edge.
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
        if (!this.following || this.pressed !== undefined) this.setUnread(true)
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
      this.setUnread(false)
      if (this.element !== null) scrollToEnd(this.element)
      return
    }
    const owner = this.anchor?.member === undefined ? undefined : this.rowByTarget.get(this.anchor.member.id)
    if (
      this.anchor !== undefined && (
        previous !== rows && owner !== undefined && owner !== this.anchor.key ||
        previousFirst !== undefined && this.firstKey !== previousFirst && this.rowKeys.has(previousFirst)
      )
    ) {
      // Rows were inserted above the reader, or a split/backfill moved the reader's member to
      // another entry (content above it changed, content below did not). The collection rebuild
      // follows this commit. ResizeObserver restores before paint when the sizer grows; the
      // tracked animation frame is the fallback.
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
  ) {
    const member = 'member' in value && typeof value.member === 'object' && value.member !== null &&
      'id' in value.member && typeof value.member.id === 'string' &&
      'offset' in value.member && typeof value.member.offset === 'number' && Number.isFinite(value.member.offset) &&
      'summary' in value.member && typeof value.member.summary === 'boolean'
      ? { id: value.member.id, offset: value.member.offset, summary: value.member.summary }
      : undefined
    return { _tag: 'Reading', entry: value.entry, offset: value.offset, scrollTop: value.scrollTop, member }
  }
  return { _tag: 'Following' }
}

export function saveScrollAnchor(key: string, anchor: ScrollAnchor) {
  if (typeof window !== 'undefined') {
    window.localStorage.setItem(`fractal-ui.embrace:${key}:conversation.anchor`, JSON.stringify(anchor))
  }
}
