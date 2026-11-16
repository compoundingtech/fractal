import * as React from 'react'
import type { Command, Drop, Side } from './model.ts'
import { paneKey, subjectOfRef } from './subject.ts'

/**
 * Everything that can open in a tab goes through here: one `open(ref)` verb and its drag counterpart, so a
 * surface gets click, Cmd-click, drag and the keyboard together and cannot implement only half. The kit
 * receives the resulting props and never learns a route, a pane key or a store.
 */
export interface OpenGesture {
  /** Add the tab without showing it. */
  readonly background?: boolean
  /** Land at this drop: a strip position or a split edge. */
  readonly at?: Drop
  /** A new split beside the focused group: the keyboard's way to reach an edge. */
  readonly split?: Side
}

export type Open = (ref: string, gesture?: OpenGesture) => void

/** The one command for an opening: the subject decides the kind, and the machine opens or focuses its one tab. */
export const openCommand = ({ ref, gesture = {}, focus }: {
  readonly ref: string
  readonly gesture?: OpenGesture
  readonly focus: string
}): Extract<Command, { _tag: 'Open' }> => {
  const pane = paneKey(subjectOfRef(ref))
  const at: Drop | undefined = gesture.at ?? (gesture.split === undefined ? undefined : { _tag: 'Edge', group: focus, side: gesture.split, first: false })
  if (at !== undefined) return { _tag: 'Open', pane, at }
  return gesture.background === true ? { _tag: 'Open', pane, activate: false } : { _tag: 'Open', pane }
}

/** The one drag payload. A tab being dragged carries its subject's ref too, so every drop is the same `Open`. */
export const openableDragType = 'application/x-fractal-openable'

export const readOpenableDrag = (data: Pick<DataTransfer, 'getData'>): string | undefined => {
  const ref = data.getData(openableDragType)
  return ref === '' ? undefined : ref
}

interface PointerModifiers {
  readonly metaKey: boolean
  readonly ctrlKey: boolean
}

interface KeyLike extends PointerModifiers {
  readonly key: string
  readonly altKey: boolean
  readonly shiftKey: boolean
  readonly preventDefault: () => void
}

interface DragStartLike {
  readonly dataTransfer: Pick<DataTransfer, 'setData' | 'effectAllowed'>
}

export interface OpenableProps {
  readonly draggable: true
  readonly onClick: (event: PointerModifiers) => void
  readonly onKeyDown: (event: KeyLike) => void
  readonly onDragStart: (event: DragStartLike) => void
}

/**
 * Click opens and focuses; Cmd or Ctrl-click opens behind. Enter opens, with the same modifier behind;
 * Alt+Enter opens in a split to the right and Alt+Shift+Enter in one below, the keyboard's answer to
 * dragging onto an edge. Enter without Alt is left to a host whose control already activates on it.
 */
export const openableProps = (ref: string, open: Open, options?: { readonly activatesOnEnter?: boolean }): OpenableProps => ({
  draggable: true,
  onClick: event => open(ref, event.metaKey || event.ctrlKey ? { background: true } : {}),
  onKeyDown: event => {
    if (event.key !== 'Enter') return
    if (event.altKey) {
      event.preventDefault()
      open(ref, { split: event.shiftKey ? 'below' : 'right' })
    } else if (options?.activatesOnEnter !== true) {
      event.preventDefault()
      open(ref, event.metaKey || event.ctrlKey ? { background: true } : {})
    }
  },
  onDragStart: event => {
    event.dataTransfer.setData(openableDragType, ref)
    event.dataTransfer.setData('text/plain', ref)
    event.dataTransfer.effectAllowed = 'copyMove'
  },
})

const OpenContext = React.createContext<Open | undefined>(undefined)

/** Provided once, by the workspace host. */
export const OpenProvider = OpenContext.Provider

/** The verb alone, for what is not a row: a menu item, a palette result. */
export const useOpen = (): Open => {
  const open = React.useContext(OpenContext)
  if (open === undefined) throw new Error('useOpen needs an OpenProvider')
  return open
}

/** Spread onto whatever shows a ref. */
export const useOpenable = (ref: string, options?: { readonly activatesOnEnter?: boolean }): OpenableProps => {
  const open = useOpen()
  const activatesOnEnter = options?.activatesOnEnter
  return React.useMemo(() => openableProps(ref, open, activatesOnEnter === undefined ? undefined : { activatesOnEnter }), [ref, open, activatesOnEnter])
}
