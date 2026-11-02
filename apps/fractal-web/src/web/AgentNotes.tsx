import * as React from 'react'
import { useAtom, useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import { Schema } from 'effect'
import * as stylex from '@stylexjs/stylex'
import { VisuallyHidden } from 'react-aria'
import { NoteBadge, NoteComposer, NoteList, NoteQuickAdd, useNoteQuickAdd, useNoteShortcut, type NoteQuickAddState } from '../../../../packages/fractal-ui/src/notes/index.ts'
import { Button } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Controls.tsx'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import type { SidebarActions } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'
import { geometryVars, interactionVars, motionVars, radiusVars, spaceVars, textVars, typeVars } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useDataSource, useSubjectNotes } from '../data/react.tsx'
import { persistedAtom } from '../state/persistence.ts'

const noActor = Atom.make<string | undefined>(undefined)
// The data-source registry scopes these; retained panes and quick-add share each target's draft, and a
// pane's disclosure and composer state survive switching to another agent and back.
const noteDraft = Atom.family((target: string) => Atom.make('').pipe(Atom.keepAlive))
const notesOpen = Atom.family((target: string) => Atom.make(false).pipe(Atom.keepAlive))
const composerOpen = Atom.family((target: string) => Atom.make(false).pipe(Atom.keepAlive))
/** One per-user preference: whether unmodified `n` opens the quick-add for the selected agent. */
export const noteShortcutPreference = persistedAtom({ key: 'notes.shortcut', schema: Schema.Boolean, defaultValue: true })
/** Shown in place of a missing agent title; never a raw ref. */
const fallbackTargetLabel = 'this agent'

const useNoteComposer = (target: string) => {
  const source = useDataSource()
  const feed = useSubjectNotes(target)
  const actor = useAtomValue(source.notesActor ?? noActor)
  const [draft, onDraftChange] = useAtom(noteDraft(target))
  const actions = source.noteActions
  const disabledReason = actions === undefined ? 'This source cannot write notes.'
    : actor === undefined ? 'Waiting for your signed-in identity.'
    : feed._tag !== 'Observed' || feed.freshness !== 'live' ? 'Wait for the notes connection before adding a note.'
    : undefined
  return { source, feed, actor, actions, composer: { draft, onDraftChange, disabledReason, onSubmit: async (text: string) => {
    if (actions === undefined || disabledReason !== undefined) throw new Error(disabledReason ?? 'This source cannot write notes.')
    await actions.create(target, text)
  } } }
}

const laidOut = (element: Element | null): Element | null => element !== null && element.getClientRects().length > 0 ? element : null
/**
 * Where the `n` popover anchors: the selected row's menu trigger while the sidebar shows it, otherwise the
 * agent pane's notes header (the sidebar is collapsed, or a closed phone sheet has unmounted the row).
 */
const noteAnchor = (ref: string): Element | null => {
  const escaped = CSS.escape(ref)
  return laidOut(document.querySelector(`[data-wf-agent-ref="${escaped}"] [data-agent-menu]`))
    ?? [...document.querySelectorAll(`[data-agent-notes="${escaped}"]`)].find(element => laidOut(element) !== null)
    ?? null
}

const QuickAddFor = ({ state, target, targetLabel, shortcutEnabled, onShortcutEnabledChange }: {
  readonly state: NoteQuickAddState
  readonly target: string
  readonly targetLabel: string
  readonly shortcutEnabled: boolean
  readonly onShortcutEnabledChange: (enabled: boolean) => void
}) => {
  const { composer } = useNoteComposer(target)
  return <NoteQuickAdd state={state} targetLabel={targetLabel} shortcutEnabled={shortcutEnabled} onShortcutEnabledChange={onShortcutEnabledChange} {...composer} />
}

/**
 * Workspace-scoped quick-add: one `n` binding for the selected agent, the rows' "Add note…" menu action and
 * one popover. Mount `notePopover` once inside the workspace's themed tree and merge `noteActions` into the
 * sidebar actions.
 */
export const useAgentNoteQuickAdd = ({ current, labelFor }: { readonly current: string; readonly labelFor: (ref: string) => string | undefined }) => {
  const source = useDataSource()
  const state = useNoteQuickAdd()
  const [shortcutEnabled, setShortcutEnabled] = useAtom(noteShortcutPreference)
  const supported = source.notes !== undefined
  useNoteShortcut(supported && shortcutEnabled && current !== '', () => {
    const anchor = noteAnchor(current)
    if (anchor === null) return false
    state.open(current, anchor)
    return true
  })
  const noteActions = React.useMemo<Pick<SidebarActions, 'note' | 'noteShortcut'>>(() => !supported ? {}
    : shortcutEnabled ? { note: state.open, noteShortcut: 'N' } : { note: state.open }, [supported, shortcutEnabled, state.open])
  const target = state.request?.target
  const notePopover = !supported || target === undefined ? null
    : <QuickAddFor state={state} target={target} targetLabel={labelFor(target)?.trim() || fallbackTargetLabel} shortcutEnabled={shortcutEnabled} onShortcutEnabledChange={setShortcutEnabled} />
  return { noteActions, notePopover }
}

const styles = stylex.create({
  host: { flexShrink: 0, minWidth: 0, paddingInline: spaceVars.section, paddingBlock: spaceVars.xs },
  lane: { maxWidth: geometryVars.lane, marginInline: 'auto', minWidth: 0 },
  header: { display: 'flex', alignItems: 'flex-start', gap: spaceVars.sm, minWidth: 0 },
  details: { flexGrow: 1, minWidth: 0 },
  summary: {
    display: 'flex', alignItems: 'center', gap: spaceVars.xs, minWidth: 0, minHeight: geometryVars.controlSm, width: 'fit-content', maxWidth: '100%',
    cursor: 'pointer', listStyle: 'none', color: textVars.fg, fontSize: typeVars.metaSize, lineHeight: typeVars.metaLeading, borderRadius: radiusVars.keyboard,
    outlineStyle: 'none',
    ':focus-visible': { outlineWidth: geometryVars.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing, outlineOffset: geometryVars.focusOffset },
    '::-webkit-details-marker': { display: 'none' },
  },
  chevron: { display: 'inline-flex', flexShrink: 0, color: textVars.fgMuted, transitionProperty: 'transform', transitionDuration: motionVars.fast, '@media (prefers-reduced-motion: reduce)': { transitionDuration: '0s' } },
  chevronOpen: { transform: 'rotate(90deg)' },
  heading: { minHeight: geometryVars.controlSm, display: 'flex', alignItems: 'center', color: textVars.fgMuted, fontSize: typeVars.metaSize, lineHeight: typeVars.metaLeading },
  body: { display: 'flex', flexDirection: 'column', gap: spaceVars.sm, minWidth: 0, paddingBlockEnd: spaceVars.sm },
  list: { maxHeight: geometryVars.commandMaxHeight, overflowY: 'auto' },
  status: { margin: spaceVars.zero, color: textVars.fgMuted, fontSize: typeVars.metaSize, lineHeight: typeVars.metaLeading },
})

/**
 * The agent pane's notes: one line at rest (the disclosure with its count and newest preview, and a quiet
 * **Add note**), expanding into the shared composer beside the list. The retained notes window owns truth;
 * the kit owns only input and presentation.
 */
export const AgentNotes = ({ agentRef, targetLabel }: { readonly agentRef: string; readonly targetLabel: string }) => {
  const { source, feed, actor, actions, composer } = useNoteComposer(agentRef)
  const [open, setOpen] = useAtom(notesOpen(agentRef))
  const [composing, setComposing] = useAtom(composerOpen(agentRef))
  const [announcement, setAnnouncement] = React.useState('')
  const addNote = React.useRef<HTMLButtonElement>(null)
  if (source.notes === undefined) return null
  const label = targetLabel.trim() || fallbackTargetLabel
  const collapse = () => {
    setComposing(false)
    requestAnimationFrame(() => addNote.current?.focus())
  }
  const composerNode = composing && <NoteComposer key={agentRef} {...composer} target={agentRef} targetLabel={label} showTarget={false} surface="bare" autoFocus
    onClose={collapse} onSubmit={async text => {
      await composer.onSubmit(text)
      setAnnouncement(`Note added to ${label}.`)
      collapse()
    }} />
  const observed = feed._tag === 'Observed' ? feed : undefined
  return <section aria-label="Agent notes" data-testid="agent-notes" data-agent-notes={agentRef} {...stylex.props(styles.host)}>
    <div {...stylex.props(styles.lane)}>
      <div {...stylex.props(styles.header)}>
        {observed !== undefined
          ? <details open={open} onToggle={event => setOpen(event.currentTarget.open)} data-testid="agent-notes-disclosure" {...stylex.props(styles.details)}>
            <summary data-testid="agent-notes-summary" {...stylex.props(styles.summary)}>
              <span aria-hidden="true" {...stylex.props(styles.chevron, open && styles.chevronOpen)}><Icon name="chevron-right" /></span>
              {observed.value.length === 0 ? 'Notes' : <NoteBadge notes={observed.value} interactive={false} />}
            </summary>
            <div {...stylex.props(styles.body)}>
              <div {...stylex.props(styles.list)}><NoteList notes={observed.value} author={actor}
                onArchive={observed.freshness === 'live' ? actions?.archive : undefined} /></div>
              {composerNode}
            </div>
          </details>
          : <p role="status" data-testid="agent-notes-feed-status" {...stylex.props(styles.heading, styles.details)}>{feed._tag === 'Waiting' ? 'Loading notes…' : 'Notes are not available from this gateway.'}</p>}
        {/* A gateway without notes offers nothing to add to; a loading feed still takes a draft. */}
        {!composing && feed._tag !== 'Unavailable' && <Button ref={addNote} variant="ghost" size="sm" data-testid="agent-notes-add" onPress={() => { setOpen(true); setComposing(true) }}>Add note</Button>}
      </div>
      {observed === undefined && feed._tag !== 'Unavailable' && composerNode}
      {observed?.freshness === 'stale' && <p role="status" data-testid="agent-notes-stale" {...stylex.props(styles.status)}>Notes are out of date. Waiting for the connection.</p>}
      {observed?.coverage?._tag === 'Partial' && <p role="status" data-testid="agent-notes-partial" {...stylex.props(styles.status)}>Showing notes in the current window.</p>}
      <VisuallyHidden><span role="status" data-testid="agent-notes-status">{announcement}</span></VisuallyHidden>
    </div>
  </section>
}
