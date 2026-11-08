import * as React from 'react'
import type { Direction } from '../geometry.ts'
import { neighbour, paneRects } from '../geometry.ts'
import { activeTab, currentSpace, type Command, type WorkspaceState } from '../model.ts'
import { isMac } from '../palette/core.ts'
import { useWorkspaceContext } from './context.tsx'

export type WorkspaceAction =
  | 'splitRight' | 'splitDown'
  | 'closeTab' | 'newTab' | 'reopenClosed'
  | 'nextTab' | 'previousTab'
  | 'focusLeft' | 'focusRight' | 'focusUp' | 'focusDown'
  | 'zoom' | 'switchSpace' | 'palette'

/**
 * A chord such as `Mod+D`, `Alt+ArrowLeft` or `Mod+Shift+D`. `Mod` is Cmd on a Mac and Ctrl elsewhere.
 * Letters and digits in a chord with Alt match the physical key, so they work where Alt types a symbol.
 */
export type KeyCombo = string

export type KeyBindings = Readonly<Partial<Record<WorkspaceAction, readonly KeyCombo[]>>>

/** The defaults: splits, tabs, focus, zoom and spaces as cmux binds them; the palette on Mod+K, and Ctrl+T on a Mac. */
export const defaultBindings = (mac = isMac()): KeyBindings => ({
  splitRight: ['Mod+D'],
  splitDown: ['Mod+Shift+D'],
  closeTab: ['Alt+W'],
  newTab: ['Alt+T'],
  reopenClosed: ['Alt+Shift+T'],
  nextTab: ['Alt+]', 'Ctrl+PageDown'],
  previousTab: ['Alt+[', 'Ctrl+PageUp'],
  focusLeft: ['Alt+ArrowLeft'],
  focusRight: ['Alt+ArrowRight'],
  focusUp: ['Alt+ArrowUp'],
  focusDown: ['Alt+ArrowDown'],
  zoom: ['Alt+O'],
  switchSpace: ['Alt+G'],
  palette: mac ? ['Mod+K', 'Ctrl+T'] : ['Mod+K'],
})

interface Chord {
  readonly mod: boolean
  readonly ctrl: boolean
  readonly meta: boolean
  readonly alt: boolean
  readonly shift: boolean
  readonly key: string
}

export const parseCombo = (combo: KeyCombo, mac = isMac()): Chord => {
  const parts = combo.split('+')
  // `Alt++` names the plus key.
  const key = combo.endsWith('++') ? '+' : parts[parts.length - 1]!
  const modifiers = new Set(parts.slice(0, combo.endsWith('++') ? -2 : -1).map(part => part.toLowerCase()))
  const mod = modifiers.has('mod')
  return {
    mod,
    ctrl: modifiers.has('ctrl') || (mod && !mac),
    meta: modifiers.has('meta') || modifiers.has('cmd') || (mod && mac),
    alt: modifiers.has('alt') || modifiers.has('option'),
    shift: modifiers.has('shift'),
    key,
  }
}

const physical = (key: string): string | undefined =>
  /^[a-z]$/i.test(key) ? `Key${key.toUpperCase()}` : /^[0-9]$/.test(key) ? `Digit${key}` : key === '[' ? 'BracketLeft' : key === ']' ? 'BracketRight' : undefined

export interface KeyLike {
  readonly key: string
  readonly code: string
  readonly altKey: boolean
  readonly shiftKey: boolean
  readonly metaKey: boolean
  readonly ctrlKey: boolean
}

export const matchesCombo = (event: KeyLike, combo: KeyCombo, mac = isMac()): boolean => {
  const chord = parseCombo(combo, mac)
  if (event.ctrlKey !== chord.ctrl || event.metaKey !== chord.meta || event.altKey !== chord.alt || event.shiftKey !== chord.shift) return false
  const code = physical(chord.key)
  if (chord.alt && code !== undefined) return event.code === code
  return event.key.toLowerCase() === chord.key.toLowerCase() || (code !== undefined && event.code === code)
}

export const actionFor = (event: KeyLike, bindings: KeyBindings, mac = isMac()): WorkspaceAction | undefined => {
  for (const [action, combos] of Object.entries(bindings) as [WorkspaceAction, readonly KeyCombo[] | undefined][]) {
    if (combos?.some(combo => matchesCombo(event, combo, mac)) === true) return action
  }
  return undefined
}

const focusDirection: Partial<Record<WorkspaceAction, Direction>> = { focusLeft: 'left', focusRight: 'right', focusUp: 'up', focusDown: 'down' }

/** What an action does to the workspace, when it is the model's to do. */
export const commandFor = (action: WorkspaceAction, state: WorkspaceState<unknown>): Command<unknown> | undefined => {
  const space = currentSpace(state)
  switch (action) {
    case 'splitRight': return { _tag: 'Split', side: 'right' }
    case 'splitDown': return { _tag: 'Split', side: 'bottom' }
    case 'closeTab': {
      const tab = activeTab(space)
      return tab === undefined ? undefined : { _tag: 'Close', tab: tab.id }
    }
    case 'reopenClosed': return { _tag: 'ReopenClosed' }
    case 'nextTab': return { _tag: 'Cycle', by: 1 }
    case 'previousTab': return { _tag: 'Cycle', by: -1 }
    case 'zoom': return { _tag: 'ToggleZoom' }
    default: {
      const direction = focusDirection[action]
      return direction === undefined ? undefined : { _tag: 'FocusDirection', direction }
    }
  }
}

export interface WorkspaceKeysOptions {
  readonly bindings?: KeyBindings | undefined
  /** Actions the host performs: a new tab, switching space, the palette. Returning `false` lets the key through. */
  readonly handlers?: Partial<Record<WorkspaceAction, () => boolean | void>> | undefined
  /** Moving focus past the layout's edge, such as left of the first pane to a sidebar. */
  readonly onEdge?: ((direction: Direction) => void) | undefined
  /** Keys the workspace must not take, such as those typed into a terminal. */
  readonly ignore?: ((event: KeyboardEvent) => boolean) | undefined
  readonly target?: EventTarget | undefined
}

/** Binds the workspace's keys on the document (or `target`), in the capture phase. */
export const useWorkspaceKeys = (options: WorkspaceKeysOptions = {}) => {
  const { store, dispatch } = useWorkspaceContext()
  const latest = React.useRef(options)
  latest.current = options
  React.useEffect(() => {
    const target = latest.current.target ?? (typeof document === 'undefined' ? undefined : document)
    if (target === undefined) return
    const mac = isMac()
    const fallback = defaultBindings(mac)
    const onKeyDown = (raw: Event) => {
      const event = raw as KeyboardEvent
      const { bindings = fallback, handlers, onEdge, ignore } = latest.current
      if (event.defaultPrevented || ignore?.(event) === true) return
      const action = actionFor(event, bindings, mac)
      if (action === undefined) return
      const handler = handlers?.[action]
      if (handler !== undefined) {
        if (handler() === false) return
      } else {
        const state = store.getState()
        const command = commandFor(action, state)
        if (command === undefined) return
        if (command._tag === 'FocusDirection') {
          const space = currentSpace(state)
          if (space.local.zoomed === null && neighbour(paneRects(space.layout), space.local.focus, command.direction) === undefined) {
            if (onEdge === undefined) return
            event.preventDefault()
            event.stopPropagation()
            onEdge(command.direction)
            return
          }
        }
        dispatch(command)
      }
      event.preventDefault()
      event.stopPropagation()
    }
    target.addEventListener('keydown', onKeyDown, true)
    return () => target.removeEventListener('keydown', onKeyDown, true)
  }, [store, dispatch])
}
