import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { useLocale } from 'react-aria'
import { Button, Collection, ListLayout, Tree, TreeItem, TreeItemContent, Virtualizer, type Key, type TreeItemRenderProps } from 'react-aria-components'
import { borderVars as border, geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink } from '../composition-tokens.stylex'
import { measuredListLayoutOptions, scrollToLayoutKey } from '../composition/virtual-layout'

export interface SidebarTreeItem<T> {
  readonly id: Key
  readonly depth: number
  readonly children?: readonly T[]
  /** Accessible label and RAC typeahead text. */
  readonly textValue: string
}
export interface SidebarTreeHandle { reveal(key: Key): void }
export interface SidebarTreeProps<T extends SidebarTreeItem<T>> {
  readonly items: readonly T[]
  readonly renderRow: (item: T, state: { depth: number; expanded: boolean; pinned: boolean }) => React.ReactNode
  readonly expandedKeys: ReadonlySet<Key>
  readonly onExpandedChange: (keys: Set<Key>) => void
  readonly onAction?: (key: Key) => void
  readonly 'aria-label': string
  readonly ref?: React.Ref<SidebarTreeHandle>
  readonly style?: stylex.StyleXStyles
  readonly onRowContextMenu?: (item: T, event: React.MouseEvent) => void
  /** Comparison control; normal trees leave this enabled. */
  readonly stickyAncestors?: boolean
}
type Pin<T> = { item: T; height: number; offset: number; breadcrumb?: readonly T[] }
type Snapshot<T> = { pins: readonly Pin<T>[]; height: number; width: number; top?: Key; measurements: readonly T[] }

/** RAC owns the sole accessible tree; sticky rows are pointer-only visual proxies. */
export function SidebarTree<T extends SidebarTreeItem<T>>(props: SidebarTreeProps<T>) {
  const { direction } = useLocale()
  const layout = React.useMemo(() => new ListLayout<unknown>(measuredListLayoutOptions), [])
  const tree = React.useRef<HTMLDivElement>(null)
  const navigation = React.useRef<TreeItemRenderProps['state'] | undefined>(undefined)
  const frame = React.useRef<number | undefined>(undefined)
  const pending = React.useRef<Key | undefined>(undefined)
  const awaitingExpansion = React.useRef(false)
  const settling = React.useRef({ signature: '', frames: 0, attempts: 0 })
  const measurementBank = React.useRef<HTMLDivElement>(null)
  const [snapshot, setSnapshot] = React.useState<Snapshot<T>>({ pins: [], height: 0, width: 0, measurements: [] })
  const model = React.useMemo(() => {
    const all = new Map<Key, T>()
    const parents = new Map<Key, readonly T[]>()
    const visible: T[] = []
    const walk = (items: readonly T[], ancestors: readonly T[], shown: boolean) => {
      for (const item of items) {
        all.set(item.id, item)
        parents.set(item.id, ancestors)
        if (shown) visible.push(item)
        walk(item.children ?? [], [...ancestors, item], shown && props.expandedKeys.has(item.id))
      }
    }
    walk(props.items, [], true)
    return { all, parents, visible }
  }, [props.items, props.expandedKeys])

  // All viewport reads are batched here. Geometry for rows comes only from RAC layout infos.
  const measure = React.useEffectEvent(() => {
    const viewport = tree.current
    if (viewport === null || layout.virtualizer === null) return
    const scroll = viewport.scrollTop
    const height = viewport.clientHeight
    const width = viewport.clientWidth
    const rows = model.visible.flatMap(item => {
      const info = layout.getLayoutInfo(item.id)
      return info == null ? [] : [{ item, rect: info.rect, estimated: info.estimatedSize }]
    })
    const top = rows.find(row => row.rect.maxY > scroll)
    const ancestors = top === undefined ? [] : model.parents.get(top.item.id) ?? []
    const measurementCandidates = new Map<Key, T>()
    for (const item of [...ancestors, ...(pending.current === undefined ? [] : model.parents.get(pending.current) ?? [])]) {
      if (layout.getLayoutInfo(item.id)?.estimatedSize) measurementCandidates.set(item.id, item)
    }
    let candidates = ancestors.flatMap(item => {
      const row = rows.find(row => row.item.id === item.id)
      return row === undefined || row.estimated || row.rect.y >= scroll ? [] : [{ item, height: row.rect.height, offset: 0 }]
    })
    if (props.stickyAncestors === false) candidates = []
    const budget = height * 0.4
    let pins: Pin<T>[] = candidates
    // Retain deepest rows. The breadcrumb uses the real upper row's measured height.
    if (pins.length > 4 || pins.reduce((sum, pin) => sum + pin.height, 0) > budget) {
      let keep = Math.min(3, pins.length - 1)
      while (keep > 0 && pins.slice(-keep).reduce((sum, pin) => sum + pin.height, pins[0]!.height) > budget) keep--
      const compressed = pins.slice(0, pins.length - keep)
      pins = [{ ...compressed[compressed.length - 1]!, height: Math.min(pins[0]!.height, budget), breadcrumb: compressed.map(pin => pin.item) }, ...pins.slice(pins.length - keep)]
    }
    let slot = 0
    pins = pins.map((pin, index) => {
      // The first row outside this ancestor's subtree pushes the entire remaining stack.
      const boundary = rows.find(row => row.rect.y > scroll && row.item.depth <= pin.item.depth && !(model.parents.get(row.item.id) ?? []).some(parent => parent.id === pin.item.id))
      const remaining = pins.slice(index).reduce((sum, row) => sum + row.height, 0)
      const offset = Math.min(0, (boundary?.rect.y ?? Infinity) - scroll - slot - remaining)
      const result = { ...pin, offset }
      slot += pin.height
      return result
    })
    const measurements = [...measurementCandidates.values()]
    const next = { pins, height: pins.reduce((sum, pin) => sum + pin.height, 0), width, top: top?.item.id, measurements }
    setSnapshot(previous => previous.top === next.top && previous.height === next.height && previous.width === width && previous.measurements.length === measurements.length && previous.measurements.every((item, index) => item === measurements[index]) && previous.pins.length === pins.length && previous.pins.every((pin, index) => pin.item === pins[index]!.item && pin.height === pins[index]!.height && pin.offset === pins[index]!.offset && pin.breadcrumb?.map(item => item.id).join(',') === pins[index]!.breadcrumb?.map(item => item.id).join(',')) ? previous : next)
    const key = pending.current
    if (key !== undefined && model.all.has(key)) {
      // Retry while ancestors/data/observed sizes settle, including late ResizeObserver deliveries.
      const ancestors = model.parents.get(key) ?? []
      if (ancestors.some(item => !props.expandedKeys.has(item.id))) {
        if (!awaitingExpansion.current) { pending.current = undefined; return }
        const expanded = new Set(props.expandedKeys)
        for (const ancestor of ancestors) expanded.add(ancestor.id)
        props.onExpandedChange(expanded)
        return
      }
      awaitingExpansion.current = false
      const info = layout.getLayoutInfo(key)
      if (info != null) {
        const inset = props.stickyAncestors === false ? 0 : Math.min(budget, ancestors.reduce((sum, parent) => { const info = layout.getLayoutInfo(parent.id); return sum + (info?.estimatedSize === false ? info.rect.height : 0) }, 0))
        const signature = `${key}:${info.rect.y}:${info.rect.height}:${inset}`
        settling.current = { signature, frames: signature === settling.current.signature ? settling.current.frames + 1 : 0, attempts: settling.current.attempts + 1 }
        if ((settling.current.frames < 2 || measurements.length > 0) && settling.current.attempts < 8) { schedule(); return }
        if (info.rect.y < scroll + inset || info.rect.maxY > scroll + height) scrollToLayoutKey(layout, key, viewport, inset)
      }
    }
  })
  const schedule = React.useCallback(() => {
    if (frame.current !== undefined) return
    frame.current = requestAnimationFrame(() => { frame.current = undefined; measure() })
  }, [])
  const reveal = React.useEffectEvent((key: Key) => {
    pending.current = key
    awaitingExpansion.current = true
    settling.current = { signature: '', frames: 0, attempts: 0 }
    const expanded = new Set(props.expandedKeys)
    for (const ancestor of model.parents.get(key) ?? []) expanded.add(ancestor.id)
    props.onExpandedChange(expanded)
    schedule()
  })
  React.useImperativeHandle(props.ref, () => ({ reveal }), [])
  React.useLayoutEffect(() => {
    schedule()
    const viewport = tree.current
    if (viewport === null) return
    const observer = new ResizeObserver(schedule)
    observer.observe(viewport)
    const mutations = new MutationObserver(() => {
      for (const row of viewport.querySelectorAll('[data-sidebar-tree-key]')) observer.observe(row)
      schedule()
    })
    mutations.observe(viewport, { childList: true, subtree: true })
    for (const row of viewport.querySelectorAll('[data-sidebar-tree-key]')) observer.observe(row)
    return () => { observer.disconnect(); mutations.disconnect(); if (frame.current !== undefined) cancelAnimationFrame(frame.current); frame.current = undefined }
  }, [schedule])
  React.useLayoutEffect(() => {
    const bank = measurementBank.current
    if (bank === null) return
    const observer = new ResizeObserver(entries => {
      for (const entry of entries) {
        const id = (entry.target as HTMLElement).dataset.measureKey
        const item = snapshot.measurements.find(item => String(item.id) === id)
        if (item === undefined || layout.virtualizer === null) continue
        const size = layout.getContentSize().copy()
        size.width = entry.contentRect.width
        size.height = entry.contentRect.height
        layout.virtualizer.updateItemSize(item.id, size)
      }
      schedule()
    })
    for (const row of bank.children) observer.observe(row)
    return () => observer.disconnect()
  }, [snapshot.measurements, layout, schedule, props.renderRow])
  React.useLayoutEffect(schedule, [model, schedule])
  const focusReal = (key: Key) => {
    reveal(key)
    navigation.current?.selectionManager.setFocused(true)
    navigation.current?.selectionManager.setFocusedKey(key)
    props.onAction?.(key)
  }
  const keyboard = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const state = navigation.current
    if (state === undefined || !(event.target instanceof HTMLElement) || event.target.getAttribute('role') !== 'row' || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return
    const expand = direction === 'rtl' ? 'ArrowLeft' : 'ArrowRight'
    const collapse = direction === 'rtl' ? 'ArrowRight' : 'ArrowLeft'
    if (event.key !== expand && event.key !== collapse) return
    const key = state.selectionManager.focusedKey
    const node = key === null ? undefined : state.collection.getItem(key)
    if (node == null) return
    const child = [...state.collection.getChildren?.(node.key) ?? []].find(node => node.type === 'item')
    let destination: Key | undefined
    if (event.key === expand) {
      if (child === undefined) return
      if (state.expandedKeys.has(node.key)) destination = child.key
      else state.toggleKey(node.key)
    } else if (child !== undefined && state.expandedKeys.has(node.key)) state.toggleKey(node.key)
    else {
      const parent = node.parentKey == null ? undefined : state.collection.getItem(node.parentKey)
      if (parent?.type !== 'item') return
      destination = parent.key
    }
    event.preventDefault()
    event.stopPropagation()
    if (destination !== undefined) {
      state.selectionManager.setFocused(true)
      state.selectionManager.setFocusedKey(destination)
    }
  }
  const row = (item: T, expanded: boolean, pinned: boolean) => <div {...stylex.props(styles.row)} style={{ paddingInlineStart: `calc(${s.lg} * ${item.depth})` }}>
    {Array.from({ length: item.depth }, (_, depth) => <span key={depth} aria-hidden {...stylex.props(styles.guide)} style={{ insetInlineStart: `calc(${s.lg} * ${depth} + ${s.sm})` }} />)}
    {item.children?.length ? pinned ? <span data-pinned-chevron {...stylex.props(styles.chevron)}>{expanded ? '⌄' : '›'}</span> : <Button slot="chevron" {...stylex.props(styles.chevron)}>{expanded ? '⌄' : '›'}</Button> : null}
    <div {...stylex.props(styles.content)}>{props.renderRow(item, { depth: item.depth, expanded, pinned })}</div>
  </div>
  const renderItem = (item: T): React.ReactElement => <TreeItem id={item.id} textValue={item.textValue} data-sidebar-tree-key={String(item.id)} onContextMenu={event => props.onRowContextMenu?.(item, event)} className={({ isFocusVisible }) => stylex.props(styles.item, isFocusVisible && styles.focused).className ?? ''}>
    <TreeItemContent>{({ isExpanded }) => row(item, isExpanded, false)}</TreeItemContent>
    <Collection items={item.children ?? []} dependencies={[props.renderRow, props.onRowContextMenu]}>{renderItem}</Collection>
  </TreeItem>
  return <div {...stylex.props(styles.root, props.style)} onKeyDownCapture={event => { pending.current = undefined; keyboard(event) }} onScrollCapture={schedule} onWheelCapture={() => { pending.current = undefined }} onPointerDownCapture={() => { pending.current = undefined }} onFocusCapture={event => {
    const target = (event.target as HTMLElement).closest('[data-sidebar-tree-key]')
    const key = target?.getAttribute('data-sidebar-tree-key')
    const item = model.visible.find(item => String(item.id) === key)
    if (item !== undefined) { pending.current = item.id; awaitingExpansion.current = false; schedule() }
  }}>
    <Virtualizer layout={layout} shouldObserveItemSize><Tree ref={tree} aria-label={props['aria-label']} items={props.items} dependencies={[props.renderRow, props.onRowContextMenu]} selectionMode="none" expandedKeys={props.expandedKeys} onExpandedChange={props.onExpandedChange} onAction={props.onAction} className={({ state }) => { navigation.current = state; return stylex.props(styles.tree).className ?? '' }}>{renderItem}</Tree></Virtualizer>
    <div aria-hidden="true" data-sidebar-pinned-stack data-top-key={snapshot.top} {...stylex.props(styles.stack)} style={{ height: snapshot.height }}>
      {snapshot.pins.map(pin => <div key={pin.item.id} {...stylex.props(styles.pinSlot)} style={{ height: pin.height }}><div data-pinned-key={String(pin.item.id)} data-pinned-ancestors={(pin.breadcrumb ?? [pin.item]).map(item => item.id).join(',')} {...stylex.props(styles.pin)} style={{ height: pin.height, transform: `translateY(calc(${pin.offset} * ${g.hairline}))` }} onClick={event => {
        const target = event.target as HTMLElement
        if (target.closest('[data-pinned-toggle]')) { const keys = new Set(props.expandedKeys); keys.has(pin.item.id) ? keys.delete(pin.item.id) : keys.add(pin.item.id); props.onExpandedChange(keys) }
        else focusReal(pin.item.id)
      }} onContextMenu={event => props.onRowContextMenu?.(pin.item, event)}>
        <div inert {...stylex.props(styles.copy)}>{pin.breadcrumb ? <div data-sidebar-breadcrumb {...stylex.props(styles.breadcrumb)}>{pin.breadcrumb.map(item => item.textValue).join(' / ')}</div> : row(pin.item, props.expandedKeys.has(pin.item.id), true)}</div>
        {pin.item.children?.length && !pin.breadcrumb ? <span data-pinned-toggle {...stylex.props(styles.toggleProxy)} style={{ insetInlineStart: `calc(${s.lg} * ${pin.item.depth})` }} /> : null}
      </div></div>)}
    </div>
    <div ref={measurementBank} aria-hidden inert {...stylex.props(styles.measurementBank)} style={{ width: snapshot.width }}>{snapshot.measurements.map(item => <div key={item.id} data-measure-key={String(item.id)}>{row(item, props.expandedKeys.has(item.id), true)}</div>)}</div>
  </div>
}
const styles = stylex.create({
  root: { position: 'relative', height: '100%', minHeight: 0, minWidth: 0, overflow: 'hidden', color: ink.fg },
  tree: { height: '100%', minHeight: 0, overflowY: 'auto', outline: 'none' },
  item: { outline: 'none' },
  focused: { outlineStyle: 'solid', outlineWidth: g.focusRing, outlineOffset: `calc(-1 * ${g.focusRing})`, outlineColor: ink.fgMuted },
  row: { position: 'relative', display: 'flex', alignItems: 'center', minHeight: g.controlSm, minWidth: 0, boxSizing: 'border-box' },
  guide: { position: 'absolute', top: 0, bottom: 0, width: g.hairline, backgroundColor: border.borderStrong, pointerEvents: 'none' },
  chevron: { display: 'flex', alignItems: 'center', justifyContent: 'center', width: g.controlSm, height: g.controlSm, flexShrink: 0, padding: 0, borderWidth: 0, backgroundColor: 'transparent', color: ink.fgMuted },
  content: { flex: 1, minWidth: 0 },
  stack: { position: 'absolute', top: 0, insetInline: 0, zIndex: 1, pointerEvents: 'none', overflow: 'hidden', maxHeight: '40%' },
  pinSlot: { position: 'relative', overflow: 'hidden', pointerEvents: 'none' },
  measurementBank: { position: 'absolute', top: 0, insetInlineStart: 0, visibility: 'hidden', pointerEvents: 'none' },
  pin: { position: 'relative', overflow: 'hidden', backgroundColor: { default: surface.sidebar, ':hover': surface.rowHover }, pointerEvents: 'auto', cursor: 'pointer' },
  copy: { height: '100%', pointerEvents: 'none' },
  toggleProxy: { position: 'absolute', top: 0, width: g.controlSm, height: '100%' },
  breadcrumb: { height: '100%', display: 'flex', alignItems: 'center', paddingInline: s.md, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis', fontSize: 'inherit', color: ink.fgMuted },
})
