import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { useSeparator } from 'react-aria'
import { borderVars as border, geometryVars as g, interactionVars as interaction } from '../composition-tokens.stylex'

/** Changes dock bounds (px): narrow enough to keep the thread lane, wide enough for a split diff. */
export const dockedSplitBounds = { min: 320, max: 720, defaultSize: 440 } as const

/** Device-local storage entry for one docked split; same `composition.` namespace as `ResizableSplit`. */
export const dockedSplitStorageKey = (persistKey: string) => `composition.${persistKey}`

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, Math.round(value)))

/** The persisted width for `persistKey`, clamped to the bounds; the default when nothing valid is stored. */
export function readDockedSize(persistKey: string, { min, max, defaultSize }: { readonly min: number; readonly max: number; readonly defaultSize: number }) {
  let stored: number | undefined
  try {
    const raw = window.localStorage.getItem(dockedSplitStorageKey(persistKey))
    stored = raw === null ? undefined : Number(raw)
  } catch { stored = undefined }
  return stored !== undefined && Number.isFinite(stored) && stored > 0 ? clamp(stored, min, max) : clamp(defaultSize, min, max)
}

export interface DockedSplitProps {
  /** Workspace-scoped key: each key keeps its own width; changing it restores that key's width. */
  readonly persistKey: string
  /** Accessible name of the separator, e.g. "Resize changes". */
  readonly label: string
  /** The main pane. It keeps its DOM position whether or not a dock is shown, so its state and portals survive toggling. */
  readonly children: React.ReactNode
  /** The right-docked panel; `undefined` renders no separator and no dock. */
  readonly dock?: React.ReactNode
  readonly min?: number
  readonly max?: number
  readonly defaultSize?: number
  /** Enter on the separator: the host hides the dock. */
  readonly onCollapse?: () => void
}

/**
 * Main pane with a right-docked, resizable panel. The width lives in the `--dock-size` custom property and the
 * separator's `aria-valuenow`, both written to the DOM directly: a drag writes them at most once per frame and never
 * re-renders the host or the main pane; pointerup and every key press persist the width per `persistKey`.
 * Keyboard: ←/→ ±8px (Shift ±32px; ← widens the dock), Home/End min/max, Enter collapses, double-click resets.
 */
export function DockedSplit({ persistKey, label, children, dock, min = dockedSplitBounds.min, max = dockedSplitBounds.max, defaultSize = dockedSplitBounds.defaultSize, onCollapse }: DockedSplitProps) {
  const rootRef = React.useRef<HTMLDivElement>(null)
  const separatorRef = React.useRef<HTMLDivElement>(null)
  const sizeRef = React.useRef<number | undefined>(undefined)
  const dockId = React.useId()
  const docked = dock !== undefined
  const { separatorProps } = useSeparator({ orientation: 'vertical', elementType: 'div' })

  const apply = React.useCallback((size: number) => {
    sizeRef.current = size
    rootRef.current?.style.setProperty('--dock-size', `${size}px`)
    const separator = separatorRef.current
    if (separator !== null) {
      separator.setAttribute('aria-valuenow', String(size))
      separator.setAttribute('aria-valuetext', `${size} pixels`)
    }
  }, [])
  const persist = React.useCallback((size: number) => {
    try { window.localStorage.setItem(dockedSplitStorageKey(persistKey), String(size)) } catch { /* persistence is best-effort */ }
  }, [persistKey])
  const commit = (next: number) => { const size = clamp(next, min, max); apply(size); persist(size) }

  // Before paint, every time the dock appears or the key/bounds change: restore that key's width.
  React.useLayoutEffect(() => {
    if (docked) apply(readDockedSize(persistKey, { min, max, defaultSize }))
  }, [docked, persistKey, min, max, defaultSize, apply])

  const current = () => sizeRef.current ?? readDockedSize(persistKey, { min, max, defaultSize })

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return
    event.preventDefault()
    const node = event.currentTarget
    const pointerId = event.pointerId
    const startX = event.clientX
    const before = current()
    let latest = before
    let frame: number | undefined
    let finished = false
    node.setPointerCapture(pointerId)
    rootRef.current?.setAttribute('data-resizing', '')
    const move = (moveEvent: PointerEvent) => {
      latest = clamp(before + startX - moveEvent.clientX, min, max)
      if (frame === undefined) frame = window.requestAnimationFrame(() => { frame = undefined; apply(latest) })
    }
    const finish = (cancelled: boolean) => {
      if (finished) return
      finished = true
      if (frame !== undefined) window.cancelAnimationFrame(frame)
      node.removeEventListener('pointermove', move)
      node.removeEventListener('pointerup', up)
      node.removeEventListener('pointercancel', cancel)
      node.removeEventListener('lostpointercapture', cancel)
      rootRef.current?.removeAttribute('data-resizing')
      if (cancelled) apply(before)
      else { apply(latest); persist(latest) }
      if (node.hasPointerCapture(pointerId)) node.releasePointerCapture(pointerId)
    }
    const up = () => finish(false)
    const cancel = () => finish(true)
    node.addEventListener('pointermove', move)
    node.addEventListener('pointerup', up)
    node.addEventListener('pointercancel', cancel)
    node.addEventListener('lostpointercapture', cancel)
  }

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 32 : 8
    if (event.key === 'ArrowLeft') { event.preventDefault(); commit(current() + step) }
    else if (event.key === 'ArrowRight') { event.preventDefault(); commit(current() - step) }
    else if (event.key === 'Home') { event.preventDefault(); commit(min) }
    else if (event.key === 'End') { event.preventDefault(); commit(max) }
    else if (event.key === 'Enter' && onCollapse !== undefined) { event.preventDefault(); onCollapse() }
  }

  return <div ref={rootRef} data-docked-split={persistKey} {...stylex.props(styles.root)}>
    <div {...stylex.props(styles.main)}>{children}</div>
    {docked ? <>
      <div
        {...separatorProps}
        ref={separatorRef}
        tabIndex={0}
        aria-label={label}
        aria-controls={dockId}
        aria-valuemin={min}
        aria-valuemax={max}
        data-resizer={persistKey}
        onPointerDown={onPointerDown}
        onKeyDown={onKeyDown}
        onDoubleClick={() => commit(defaultSize)}
        {...stylex.props(styles.separator)}
      />
      <div id={dockId} data-dock {...stylex.props(styles.dock)}>{dock}</div>
    </> : null}
  </div>
}

const styles = stylex.create({
  root: { display: 'flex', flex: '1 1 0', minWidth: 0, minHeight: 0, ':is([data-resizing])': { cursor: 'col-resize', userSelect: 'none' } },
  main: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0 },
  separator: { position: 'relative', zIndex: 1, flexShrink: 0, width: g.hairline, alignSelf: 'stretch', touchAction: 'none', cursor: 'col-resize', backgroundColor: border.border, '::after': { content: '""', position: 'absolute', insetInline: `calc(-1 * ${g.separatorHitSlop})`, insetBlock: 0 }, ':hover': { backgroundColor: border.borderStrong }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: `calc(-1 * ${g.focusOffset})` } },
  dock: { display: 'flex', flexDirection: 'column', flexShrink: 0, width: 'var(--dock-size)', minWidth: 0, minHeight: 0, overflow: 'hidden' },
})
