import * as React from 'react'
import { Hash, Schema } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import { useAtom } from '@effect/atom-react'
import type { Agent } from '../data/source.ts'
import type { SidebarNode } from '../folders/sidebarContract.ts'
import { useMissions } from '../data/react.tsx'
import { persistedAtom } from '../state/persistence.ts'
import { sidebarRow } from '../web/sidebarRow.ts'
import { CommandPalette } from './CommandPalette.tsx'
import { afterNextPaint } from '../telemetry/ux.ts'
import { agentFolders, commandSections, leadingRows, recordVisit, switchSections, type PaletteCommand, type PaletteMode, type PaletteTarget } from './model.ts'

/** Stable gateway scope without persisting connection URLs; this is an identifier hash, not encryption. */
export const paletteRecentsKey = (scope: string): string => `palette.recents.${(Hash.string(scope) >>> 0).toString(16)}`

/** P2·G1 recents store only bounded stable target refs; names and other roster content stay live. */
const recentAgents = Atom.family((scope: string) => persistedAtom({ key: paletteRecentsKey(scope), schema: Schema.Array(Schema.String), defaultValue: [] }))

/** Records every successful navigation; opening or browsing the palette never counts as a visit. */
export const usePaletteRecents = (scope: string, current: string): readonly string[] => {
  const [recents, setRecents] = useAtom(recentAgents(scope))
  React.useEffect(() => {
    if (current !== '') setRecents(value => recordVisit(value, current))
  }, [current, setRecents])
  return recents
}

/** A 1440×980 dialog shows about 15 rows at once. */
const firstFrameRows = 16

const editableTerminal = '[aria-roledescription="terminal"]'

/**
 * Mod+K opens the palette from anywhere except a terminal, which owns Ctrl+K. Literal Ctrl+T is a tested alias:
 * browsers deliver it to the page only in standalone windows, so it is never advertised.
 */
const isPaletteChord = (event: KeyboardEvent): boolean => {
  if (event.defaultPrevented || event.isComposing || event.repeat || event.altKey || event.shiftKey) return false
  const key = event.key.toLowerCase()
  if (!(key === 'k' && (event.metaKey || event.ctrlKey)) && !(key === 't' && event.ctrlKey && !event.metaKey)) return false
  return !(event.target instanceof Element && event.target.closest(editableTerminal) !== null)
}

export interface PaletteHandle {
  /** Opens Switch mode; focus returns to `invoker` when the palette closes. */
  readonly open: (invoker: HTMLElement | null) => void
}

export interface PaletteHostProps {
  readonly agents: readonly Agent[]
  readonly tree: readonly SidebarNode[]
  readonly needsYou: ReadonlySet<string>
  readonly current: string
  readonly stale: boolean
  readonly inventoryNote: string | undefined
  readonly recents: readonly string[]
  readonly commands: readonly PaletteCommand[]
  /** Runs after the palette closed and focus returned to the invoker. */
  readonly onSelect: (target: PaletteTarget) => void
  readonly portalContainer: Element | null
  readonly shortcut: string
  /** Focus target when the invoker left the document, e.g. the header Switch button. */
  readonly fallbackFocus: React.RefObject<HTMLElement | null>
  /** Dismisses competing modal surfaces; true waits for their focus restoration before opening. */
  readonly onOpen: () => boolean
}

/**
 * Owns the open state and the global chord. The workspace never re-renders to open the palette, so opening costs
 * only the palette itself.
 */
export const PaletteHost = React.memo(React.forwardRef<PaletteHandle, PaletteHostProps>(function PaletteHost({ fallbackFocus, onSelect, onOpen, ...props }, ref) {
  const [mode, setMode] = React.useState<PaletteMode | undefined>(undefined)
  const invoker = React.useRef<Element | null>(null)
  const pending = React.useRef<PaletteTarget | undefined>(undefined)
  const wasOpen = React.useRef(false)
  const open = React.useCallback((element: Element | null) => {
    invoker.current = element
    if (onOpen()) afterNextPaint(() => setMode('Switch'))
    else setMode('Switch')
  }, [onOpen])
  React.useImperativeHandle(ref, () => ({ open }), [open])
  React.useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (!isPaletteChord(event)) return
      event.preventDefault()
      // A chord from inside the open palette closes it and keeps the original invoker.
      if (mode === undefined) open(document.activeElement)
      else setMode(undefined)
    }
    document.addEventListener('keydown', keydown)
    return () => document.removeEventListener('keydown', keydown)
  }, [mode, open])
  React.useEffect(() => {
    if (mode !== undefined) {
      wasOpen.current = true
      return
    }
    if (!wasOpen.current) return
    wasOpen.current = false
    // RAC restores its FocusScope on the next frame. Wait for that cleanup before restoring the invoker
    // and running a command that may open another modal (such as the mobile sidebar).
    const element = invoker.current
    const target = pending.current
    pending.current = undefined
    return afterNextPaint(() => {
      ;(element instanceof HTMLElement && element.isConnected ? element : fallbackFocus.current)?.focus()
      if (target !== undefined) onSelect(target)
    })
  }, [mode, fallbackFocus, onSelect])
  return mode === undefined ? null : <LiveCommandPalette {...props} mode={mode} onModeChange={setMode} onClose={() => setMode(undefined)} onSelect={target => { pending.current = target; setMode(undefined) }} />
}))

/** Live data for the palette; mounted only while open, so a closed palette costs no render work. */
function LiveCommandPalette({ onClose, mode, onModeChange, agents, tree, needsYou, current, stale, inventoryNote, recents, commands, onSelect, portalContainer, shortcut }: Omit<PaletteHostProps, 'fallbackFocus' | 'onOpen'> & {
  readonly onClose: () => void
  readonly mode: PaletteMode
  readonly onModeChange: (mode: PaletteMode) => void
}) {
  const [query, setQuery] = React.useState('')
  const missionFeed = useMissions()
  const missions = React.useMemo(() => missionFeed._tag === 'Observed' ? missionFeed.value.map(view => view.mission) : undefined, [missionFeed])
  const folders = React.useMemo(() => agentFolders(tree), [tree])
  // Not deferred: a lagging collection would let ArrowDown act on rows from the previous query.
  const sections = React.useMemo(() => mode === 'Commands'
    ? commandSections({ query, commands })
    : switchSections({ query, agents, folders, needsYou, recents, current, missions, statusLabel: agent => sidebarRow({ agent, stale, now: 0 }).statusLabel }),
  [mode, query, commands, agents, folders, needsYou, recents, current, missions, stale])
  // React Aria builds the whole collection before the virtualizer picks visible rows. The first frame builds
  // only the rows a dialog can show; the complete list follows after that frame paints.
  const [complete, setComplete] = React.useState(false)
  React.useEffect(() => afterNextPaint(() => React.startTransition(() => setComplete(true))), [])
  const shown = React.useMemo(() => complete ? sections : leadingRows(sections, firstFrameRows), [complete, sections])
  return <CommandPalette
    isOpen
    onOpenChange={open => { if (!open) onClose() }}
    mode={mode}
    onModeChange={next => { onModeChange(next); setQuery('') }}
    query={query}
    onQueryChange={setQuery}
    sections={shown}
    onSelect={row => onSelect(row.target)}
    shortcut={shortcut}
    {...(mode === 'Switch' && inventoryNote !== undefined ? { inventoryNote } : {})}
    portalContainer={portalContainer}
  />
}
