import * as React from 'react'
import { ENTITY_DRAG_TYPE, entityDragPayload, parseEntityDragPayload, type EntityKind } from '../../../../packages/fractal-ui/src/assistant-ui/entity/entity.ts'
import type { Command, Drop } from './model.ts'
import { paneKey, parsePane, subjectOfRef, subjectRef, type Subject } from './subject.ts'

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
  /** A new split beside the focused pane: the keyboard's way to reach an edge. */
  readonly split?: 'right' | 'bottom'
}

export type Open = (ref: string, gesture?: OpenGesture) => void

/** The one command for an opening: the subject decides the kind, and the machine opens or focuses its one tab. */
export const openCommand = ({ ref, gesture = {}, focus }: {
  readonly ref: string
  readonly gesture?: OpenGesture
  readonly focus: string
}): Extract<Command, { _tag: 'Open' | 'Split' }> => {
  const pane = paneKey(subjectOfRef(ref))
  if (gesture.at !== undefined) return { _tag: 'Open', ref: pane, at: gesture.at }
  // An empty pane takes the subject itself; any other splits, as a drag onto that edge would.
  if (gesture.split !== undefined) return { _tag: 'Split', pane: focus, side: gesture.split, ref: pane }
  return gesture.background === true ? { _tag: 'Open', ref: pane, activate: false } : { _tag: 'Open', ref: pane }
}

/**
 * The one drag payload, the shared entity descriptor (`ENTITY_DRAG_TYPE`): its id is the subject's ref, so every
 * drop is the same `Open`, and the hover card and the drag chip name the entity one way.
 */
export const openableDragType = ENTITY_DRAG_TYPE

const entityKindOf = (subject: Subject): EntityKind =>
  subject._tag === 'Conversation' || subject._tag === 'Terminal' ? 'agent' : subject._tag === 'Mission' ? 'mission' : 'resource'

export const openableDragItems = (ref: string, title = ref): Record<string, string> =>
  entityDragPayload({ kind: entityKindOf(subjectOfRef(ref)), id: subjectRef(subjectOfRef(ref)), title })

export const readOpenableDrag = (data: Pick<DataTransfer, 'getData'>): string | undefined =>
  parseEntityDragPayload(data.getData(openableDragType) || undefined)?.id

/** A dragged tab names its subject by its pane key; the payload carries the subject's ref. */
export const refOfPane = (pane: string): string => subjectRef(parsePane(pane))

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
      open(ref, { split: event.shiftKey ? 'bottom' : 'right' })
    } else if (options?.activatesOnEnter !== true) {
      event.preventDefault()
      open(ref, event.metaKey || event.ctrlKey ? { background: true } : {})
    }
  },
  onDragStart: event => {
    for (const [type, value] of Object.entries(openableDragItems(ref))) event.dataTransfer.setData(type, value)
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
