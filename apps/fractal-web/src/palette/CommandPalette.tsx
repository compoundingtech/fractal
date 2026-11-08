import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { CompositionIcon, EntityStatusMark } from '@fractal/ui/assistant-ui'
import type { Command, WorkspaceState } from '@fractal/ui/workspace'
import { isMac, memoryRecentStore, usePalette, useRecent, type OpenTarget, type PaletteMode, type PaletteSource, type RecentStore } from '@fractal/ui/workspace/palette'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { ThemePortal } from '../../../../packages/fractal-ui/src/assistant-ui/taste/ThemePortal.tsx'
import { borderVars as border, elevationVars as elevation, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { namingCommand, openCommands, type NamingIntent } from './dispatch.ts'
import {
  commandItems, emptyOrder, emptyWorld, leads, namingItem, paletteSections, recentItems, saidItems, spaceItems, typedOrder, worldItems,
  type HostCommand, type PaletteEntry, type PaletteRow, type PaletteWorld, type SaidHit,
} from './items.ts'

/** What st's conversation search answers; `undefined` when this gateway cannot search. */
export type SaidSearch = (query: string, signal: AbortSignal) => Promise<{
  readonly items: readonly SaidHit[]
  readonly refreshing?: boolean
  readonly incomplete_sources?: readonly string[]
} | undefined>

export interface CommandPaletteProps {
  readonly isOpen: boolean
  readonly onOpenChange: (open: boolean) => void
  /** The workspace the palette opens into, and its dispatch (the app's, which keeps one agent one tab). */
  readonly state: WorkspaceState
  readonly dispatch: (command: Command) => void
  /** The workspace's tab identity, so "here" finds an agent's tab from its terminal's ref. */
  readonly identity?: ((ref: string) => string) | undefined
  readonly world?: PaletteWorld | undefined
  /** Theme, settings, toggles, new terminal and mission, sidebar modes: the host's to run. Keep it (and `world`) referentially stable: each change re-asks the sources. */
  readonly commands?: readonly HostCommand[] | undefined
  /** Full-text search of what was said in conversations, asked from three characters. */
  readonly search?: SaidSearch | undefined
  /**
   * Opens a conversation found at what was said. Without it the conversation opens where asked, at its
   * end: the web has no find-in-conversation seam yet.
   */
  readonly onSaid?: ((ref: string, query: string, target: OpenTarget) => void) | undefined
  /** Per person and gateway (`cellRecentStore`); in memory when absent. */
  readonly recentStore?: RecentStore | undefined
  /** A full-screen sheet offering only here and tab; the narrow layout's when absent. */
  readonly phone?: boolean | undefined
}

/** The palette's open state, for the host's Mod+K / Ctrl+T handler and the phone's search button. */
export const usePaletteOpen = () => {
  const [isOpen, setOpen] = React.useState(false)
  return {
    isOpen,
    onOpenChange: setOpen,
    open: React.useCallback(() => { setOpen(true) }, []),
    toggle: React.useCallback(() => { setOpen(open => !open) }, []),
  }
}

const allTargets: readonly OpenTarget[] = ['here', 'tab', 'right', 'below']
const phoneTargets: readonly OpenTarget[] = ['here', 'tab']

/**
 * The command palette, at TUI parity (crates/fractal-tui/src/ui/glass.rs): one input over sectioned
 * rows. Empty, it lists Recent and then each section; typed, one ranked list (agents, missions, what
 * needs you, what was said, starting things, fleet, spaces). A prefix scopes it (> @ # ! /) and Alt+1
 * to Alt+8 jump to the TUI's sections. Enter opens here, Mod+Enter in a tab, Alt+Enter to the right,
 * Alt+Shift+Enter below. Renaming and making spaces ask for a name in the same input.
 */
export function CommandPalette({ isOpen, onOpenChange, ...props }: CommandPaletteProps) {
  const narrow = React.useContext(NarrowLayoutContext)
  const phone = props.phone ?? narrow
  // The themed portal's host exists only after the first commit; a palette open from the start waits for it.
  const [mounted, setMounted] = React.useState(false)
  React.useLayoutEffect(() => setMounted(true), [])
  return <ThemePortal>
    <Aria.ModalOverlay isOpen={isOpen && mounted} onOpenChange={onOpenChange} isDismissable isKeyboardDismissDisabled {...stylex.props(styles.overlay, phone && styles.overlayPhone)}>
      <Aria.Modal {...stylex.props(styles.modal, phone && styles.modalPhone)}>
        <Aria.Dialog aria-label="Command palette" data-testid="command-palette" data-phone={phone ? '' : undefined} data-shortcuts-paused="" {...stylex.props(styles.dialog)}>
          <PaletteBody {...props} phone={phone} onClose={() => onOpenChange(false)} />
        </Aria.Dialog>
      </Aria.Modal>
    </Aria.ModalOverlay>
  </ThemePortal>
}

const fallbackRecent = memoryRecentStore()
// Defaults are constants: a fresh array each render would re-ask every source, which answers with a fresh render.
const noCommands: readonly HostCommand[] = []

function PaletteBody({ state, dispatch, identity, world = emptyWorld, commands = noCommands, search, onSaid, recentStore = fallbackRecent, phone, onClose }: Omit<CommandPaletteProps, 'isOpen' | 'onOpenChange' | 'phone'> & { readonly phone: boolean; readonly onClose: () => void }) {
  const { recent, push } = useRecent({ store: recentStore })
  const [mode, setMode] = React.useState<PaletteMode>({ _tag: 'Search' })
  // A tap on the phone opens here, or in a tab when that is chosen above the list.
  const [tapTarget, setTapTarget] = React.useState<OpenTarget>('here')
  const names = React.useMemo(() => new Map(world.agents.map(agent => [agent.ref, agent.name])), [world.agents])
  const sources = React.useMemo((): readonly PaletteSource<PaletteEntry>[] => [
    {
      id: 'local',
      items: query => {
        const local = [...worldItems(world, query), ...commandItems({ state, commands }), ...spaceItems(state, query)]
        return [...local, ...recentItems(recent, local)]
      },
    },
    ...(search === undefined ? [] : [{
      id: 'said', minQuery: 3, debounceMs: 250,
      items: async (query: string, signal: AbortSignal) => {
        const found = await search(query, signal)
        return found === undefined ? [] : saidItems(query, found, names)
      },
    }]),
  ], [world, state, commands, recent, search, names])

  const choose = (item: PaletteRow, target: OpenTarget) => {
    const entry = item.action
    if (entry.key !== undefined) push(entry.key)
    const action = entry.action
    switch (action._tag) {
      case 'Name':
        setMode({ _tag: 'Naming', intent: action.intent, initial: action.intent === 'rename' ? state.spaces.find(space => space.id === state.current)?.name ?? '' : '' })
        return
      case 'Open':
        for (const command of openCommands(state, action.ref, target, identity)) dispatch(command)
        break
      case 'Said':
        if (onSaid !== undefined) onSaid(action.ref, action.query, target)
        else for (const command of openCommands(state, action.ref, target, identity)) dispatch(command)
        break
      case 'Workspace':
        for (const command of action.commands) dispatch(command)
        break
      case 'Run':
        action.run()
        break
    }
    onClose()
  }

  const targets = phone ? phoneTargets : allTargets
  const palette = usePalette<PaletteEntry>({
    sections: paletteSections,
    sources,
    targets,
    recent,
    typedOrder,
    emptyOrder,
    leads,
    mode,
    onModeChange: setMode,
    onSelect: (item, target) => choose(item, phone && target === 'here' ? tapTarget : target),
    onName: (naming, name) => {
      dispatch(namingCommand(state, naming.intent, name))
      onClose()
    },
    onClose,
    'aria-label': 'Search Fractal',
  })
  const naming = palette.mode._tag === 'Naming' ? palette.mode : undefined
  const scoped = palette.section === undefined ? undefined : paletteSections.find(section => section.id === palette.section)
  const inputProps = palette.getInputProps()

  return <>
    <div {...stylex.props(styles.field)}>
      {phone && <button type="button" aria-label="Close" onClick={onClose} {...stylex.props(styles.iconButton)}><CompositionIcon name="x" size={16} /></button>}
      <CompositionIcon name={naming === undefined ? 'search' : 'pencil'} size={16} />
      {scoped !== undefined && <span data-palette-scope={scoped.id} {...stylex.props(styles.scope)}>{scoped.label}</span>}
      <input
        {...inputProps}
        autoFocus
        aria-label={naming === undefined ? 'Search Fractal' : 'Name'}
        placeholder={naming !== undefined ? namingPrompt(naming.intent) : phone ? 'Search' : 'Search agents, missions, resources and commands'}
        {...stylex.props(styles.input, phone && styles.inputPhone)}
      />
      {palette.loading && <span role="status" {...stylex.props(styles.muted)}>Searching…</span>}
    </div>
    {phone && naming === undefined && <div role="radiogroup" aria-label="Open in" {...stylex.props(styles.tapTargets)}>
      {phoneTargets.map(target => <button key={target} type="button" role="radio" aria-checked={tapTarget === target} onClick={() => setTapTarget(target)} {...stylex.props(styles.tapTarget, tapTarget === target && styles.tapTargetOn)}>
        {target === 'here' ? 'Here' : 'New tab'}
      </button>)}
    </div>}
    <div {...palette.getListProps()} {...stylex.props(styles.list)}>
      {naming !== undefined
        ? <NamingRow label={palette.query.trim() === '' ? undefined : namingItem(state, naming.intent, palette.query.trim())} />
        : palette.groups.map(group => <div key={group.section.id} {...palette.getSectionProps(group.section)}>
          <div aria-hidden="true" {...stylex.props(styles.sectionLabel)}>{group.section.label}</div>
          {group.items.map(({ item }) => <Row key={item.id} item={item} props={palette.getItemProps(item)} />)}
        </div>)}
      {naming === undefined && palette.results.length === 0 && !palette.loading && <p {...stylex.props(styles.empty)}>
        {palette.query.trim() === '' ? 'Nothing here yet.' : 'No matches.'}
      </p>}
    </div>
    {!phone && <Hints naming={naming !== undefined} />}
  </>
}

const namingPrompt = (intent: NamingIntent) =>
  intent === 'create' ? 'A new space: type its name' : intent === 'rename' ? 'Rename this space: type its new name' : 'Duplicate this space: type the copy’s name'

function Row({ item, props }: { readonly item: PaletteRow; readonly props: React.HTMLAttributes<HTMLElement> & { readonly 'data-selected'?: string } }) {
  const { entity, icon, shortcut } = item.action
  const selected = props['data-selected'] !== undefined
  return <div {...props} data-palette-row={item.id} data-section={item.section} {...stylex.props(styles.row, selected && styles.rowSelected)}>
    <span aria-hidden={entity?.status === undefined} {...stylex.props(styles.glyph)}>
      {entity?.status !== undefined ? <EntityStatusMark status={entity.status} /> : <CompositionIcon name={icon ?? 'dot'} size={14} />}
    </span>
    <span {...stylex.props(styles.label)}>{item.label}</span>
    {item.detail !== undefined && item.detail !== '' && <span {...stylex.props(styles.detail, selected && styles.detailSelected)}>{item.detail}</span>}
    {shortcut !== undefined && <kbd {...stylex.props(styles.kbd)}>{keyCaps(shortcut)}</kbd>}
  </div>
}

function NamingRow({ label }: { readonly label: string | undefined }) {
  if (label === undefined) return <p {...stylex.props(styles.empty)}>Type a name, then Enter. Escape goes back.</p>
  return <div role="option" aria-selected data-selected="" {...stylex.props(styles.row, styles.rowSelected)}>
    <span aria-hidden="true" {...stylex.props(styles.glyph)}><CompositionIcon name="plus" size={14} /></span>
    <span {...stylex.props(styles.label)}>{label}</span>
    <kbd {...stylex.props(styles.kbd)}>↵</kbd>
  </div>
}

const keyCaps = (shortcut: string, mac = isMac()) => shortcut
  .replace('Mod+', mac ? '⌘' : 'Ctrl+')
  .replace('Shift+', mac ? '⇧' : 'Shift+')
  .replace('Alt+', mac ? '⌥' : 'Alt+')

function Hints({ naming }: { readonly naming: boolean }) {
  const mac = isMac()
  const hints: readonly (readonly [string, string])[] = naming
    ? [['↵', 'name it'], ['esc', 'back']]
    : [['↵', 'here'], [mac ? '⌘↵' : 'Ctrl+↵', 'tab'], [mac ? '⌥↵' : 'Alt+↵', 'right'], [mac ? '⌥⇧↵' : 'Alt+Shift+↵', 'below'], [mac ? '⌥1–8' : 'Alt+1–8', 'section'], ['> @ # ! /', 'scope']]
  return <div data-palette-hints {...stylex.props(styles.hints)}>
    {hints.map(([keys, label]) => <span key={label} {...stylex.props(styles.hint)}><kbd {...stylex.props(styles.kbd)}>{keys}</kbd>{label}</span>)}
  </div>
}

/** The phone top bar's way in: a search button that opens the palette as a full-screen sheet. */
export function PaletteButton({ onPress }: { readonly onPress: () => void }) {
  return <Aria.Button aria-label="Search" onPress={onPress} {...stylex.props(styles.topButton)}>
    <CompositionIcon name="search" size={16} />
  </Aria.Button>
}

const styles = stylex.create({
  overlay: {
    position: 'fixed', insetInline: s.zero, insetBlockStart: s.zero, insetBlockEnd: 'var(--wf-devbar-space, 0px)', zIndex: 100,
    display: 'flex', alignItems: 'flex-start', justifyContent: 'center', paddingBlockStart: '12vh', backgroundColor: surface.scrim,
  },
  overlayPhone: { alignItems: 'stretch', paddingBlockStart: s.zero },
  modal: {
    display: 'flex', boxSizing: 'border-box', outlineStyle: 'none',
    width: '640px', maxWidth: 'calc(100vw - 32px)', maxHeight: 'min(560px, 76vh)',
    borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.lg,
    backgroundColor: surface.raised, color: ink.fg, boxShadow: elevation.dialog, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading,
  },
  modalPhone: {
    width: '100%', maxWidth: '100%', maxHeight: '100%', borderWidth: s.zero, borderRadius: r.none,
    paddingBlockStart: 'env(safe-area-inset-top)', paddingBlockEnd: 'env(safe-area-inset-bottom)', paddingInlineStart: 'env(safe-area-inset-left)', paddingInlineEnd: 'env(safe-area-inset-right)',
  },
  dialog: { display: 'flex', flexDirection: 'column', flexGrow: 1, minWidth: s.zero, minHeight: s.zero, outlineStyle: 'none' },
  field: {
    display: 'flex', alignItems: 'center', gap: s.md, flexShrink: 0, minHeight: g.band, boxSizing: 'border-box', paddingInline: s.lg,
    color: ink.fgMuted, borderBlockEndWidth: g.hairline, borderBlockEndStyle: 'solid', borderBlockEndColor: border.border,
  },
  input: {
    flexGrow: 1, minWidth: s.zero, borderWidth: s.zero, outlineStyle: 'none', backgroundColor: 'transparent', color: ink.fg,
    fontFamily: t.fontSans, fontSize: t.bodySize, lineHeight: t.uiLeading, paddingBlock: s.md,
    '::placeholder': { color: ink.fgMuted },
  },
  // At least 16px so a phone does not zoom on focus.
  inputPhone: { fontSize: t.touchInputSize },
  scope: { flexShrink: 0, paddingInline: s.sm, borderRadius: r.sm, backgroundColor: surface.controlFill, color: ink.fg, fontSize: t.metaSize },
  iconButton: {
    display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, minWidth: g.touchTarget, minHeight: g.touchTarget,
    borderWidth: s.zero, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fgMuted, cursor: 'pointer',
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset },
  },
  tapTargets: { display: 'flex', gap: s.xs, paddingInline: s.lg, paddingBlock: s.sm, borderBlockEndWidth: g.hairline, borderBlockEndStyle: 'solid', borderBlockEndColor: border.border },
  tapTarget: {
    minHeight: g.controlMd, paddingInline: s.lg, borderWidth: s.zero, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.uiSize, cursor: 'pointer',
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset },
  },
  tapTargetOn: { backgroundColor: surface.rowActive, color: ink.fg },
  list: { flexGrow: 1, minHeight: s.zero, overflowY: 'auto', overscrollBehavior: 'contain', paddingBlock: s.xs, paddingInline: s.xs },
  sectionLabel: { paddingInline: s.md, paddingBlockStart: s.md, paddingBlockEnd: s.xs, color: ink.fgMuted, fontSize: t.metaSize, fontWeight: t.weightMedium },
  row: {
    display: 'flex', alignItems: 'center', gap: s.md, minHeight: g.menuRow, boxSizing: 'border-box', paddingInline: s.md, borderRadius: r.sm, cursor: 'pointer',
    '@media (max-width: 899px)': { minHeight: g.touchTarget },
  },
  rowSelected: { backgroundColor: surface.rowActive },
  glyph: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, width: '16px', color: ink.fgMuted },
  label: { flexShrink: 1, minWidth: s.zero, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  detail: { flexShrink: 2, flexGrow: 1, minWidth: s.zero, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fgMuted, fontSize: t.metaSize },
  // The selected row's wash would take muted text under 4.5:1.
  detailSelected: { color: ink.fgSoft },
  kbd: { flexShrink: 0, paddingInline: s.xs, borderRadius: r.keyboard, backgroundColor: surface.controlFill, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.denseSize },
  muted: { flexShrink: 0, color: ink.fgMuted, fontSize: t.metaSize },
  empty: { margin: s.zero, paddingInline: s.md, paddingBlock: s.lg, color: ink.fgMuted },
  hints: {
    display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: s.lg, rowGap: s.xs, flexShrink: 0, paddingInline: s.lg, paddingBlock: s.sm,
    color: ink.fgMuted, fontSize: t.denseSize, borderBlockStartWidth: g.hairline, borderBlockStartStyle: 'solid', borderBlockStartColor: border.border,
  },
  hint: { display: 'inline-flex', alignItems: 'center', gap: s.xs },
  topButton: {
    display: 'inline-flex', alignItems: 'center', justifyContent: 'center', minWidth: g.touchTarget, minHeight: g.touchTarget,
    borderWidth: s.zero, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fgMuted, cursor: 'pointer', outlineStyle: 'none',
    ':is([data-hovered])': { color: ink.fg },
    ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset },
  },
})
