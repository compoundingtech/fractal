import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Dialog, DialogTrigger, Heading, Input, Label, Modal, ModalOverlay, TextField, type Key } from 'react-aria-components'
import type { SubjectNote } from '@fractal/client/schema'
import { NoteBadge, NoteQuickAdd, useNoteQuickAdd, useNoteShortcut } from './index'
import { agentNote, diffNote, plannerNote, plannerTarget, publishedNote, target } from './notes-fixtures'
import { NotesFrame, expectNoAudiencePresentation, notesFrameArgTypes, notesFrameArgs, notesStoryStyles as frame, type NotesFrameArgs } from './notes-story-frame'
import { Button } from '../assistant-ui/composition/Controls'
import { NarrowLayoutContext } from '../assistant-ui/composition/NarrowLayout'
import { greenFindings } from '../assistant-ui/composition/no-green-assertion'
import { MilestoneAgentRow } from '../assistant-ui/sidebar/MilestoneAgentRow'
import { SidebarTree, type SidebarTreeItem } from '../assistant-ui/sidebar/SidebarTree'
import type { SidebarActions, SidebarAgentRow as Row } from '../assistant-ui/sidebar/model'
import { ThemePortal } from '../assistant-ui/taste/ThemePortal'
import { borderVars as border, elevationVars as elevation, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as text, typeVars as t } from '../assistant-ui/composition-tokens.stylex'

const storyNow = Date.UTC(2026, 9, 14, 16, 30, 0)
const nightlyTarget = 'agent/example/nightly'
const baseRow: Row = {
  ref: target, id: 'reviewer', title: 'Reviewer', status: 'idle', statusLabel: 'Idle', host: 'build-1', freshness: 'live',
  usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Unknown' }, children: [],
}
/** "Nightly checks" starts with `n`: typeahead from the tree must land there instead of opening a note. */
const rows: readonly Row[] = [baseRow, { ...baseRow, ref: plannerTarget, id: 'planner', title: 'Planner' }, { ...baseRow, ref: nightlyTarget, id: 'nightly', title: 'Nightly checks' }]
const labels: Readonly<Record<string, string>> = Object.fromEntries(rows.map(row => [row.ref, row.title]))
interface AgentItem extends SidebarTreeItem<AgentItem> { readonly row: Row }
const items: readonly AgentItem[] = rows.map((row): AgentItem => ({ id: row.ref, depth: 0, textValue: row.title, row, disclosurePlacement: 'trailing' }))
const noneExpanded: ReadonlySet<Key> = new Set()
/** How long the planner's first create stays in flight before it fails. */
const heldCreateMs = 2000
/** The selected row's menu trigger, as the web workspace anchors the popover. */
const anchorFor = (root: HTMLElement | null, ref: string): Element | null => root?.querySelector(`[data-wf-agent-ref="${CSS.escape(ref)}"] [data-agent-menu]`) ?? null

interface PopoverArgs extends NotesFrameArgs {
  /** Phone layout: touch rows; the menu item is the entry. */
  readonly touch?: boolean
}

/**
 * UX variant B — keyboard `n` quick-add on the kit parts the web workspace uses: `useNoteShortcut` binds
 * `n` once, `useNoteQuickAdd` holds one request, and `NoteQuickAdd` is the popover. The selected sidebar
 * row's "Add note…" menu item opens the same popover; the popover's checkbox turns `n` off. `n` is left to
 * fields, editors, the tree's typeahead, open menus, dialogs and IME composition. While a create is in flight
 * the popover refuses Escape and outside presses, so its outcome reaches the open composer.
 */
function KeyboardPopoverStory({ scheme, width, negativeControl, touch = false }: PopoverArgs) {
  const [notes, setNotes] = React.useState<Readonly<Record<string, readonly SubjectNote[]>>>({ [target]: [diffNote, agentNote], [plannerTarget]: [plannerNote], [nightlyTarget]: [] })
  const [selected, setSelected] = React.useState<string>(target)
  const [drafts, setDrafts] = React.useState<Readonly<Record<string, string>>>({})
  const [shortcutOn, setShortcutOn] = React.useState(true)
  const [commandLine, setCommandLine] = React.useState('')
  const root = React.useRef<HTMLDivElement>(null)
  const quick = useNoteQuickAdd()
  // The planner's first create stays in flight for a while, then fails: long enough to try dismissing it.
  const holdPlanner = React.useRef(true)
  const openOnSelection = () => {
    const anchor = anchorFor(root.current, selected)
    if (anchor === null) return false
    quick.open(selected, anchor)
    return true
  }
  useNoteShortcut(shortcutOn && !negativeControl, openOnSelection)
  const latestOpen = React.useRef(openOnSelection)
  React.useLayoutEffect(() => { latestOpen.current = openOnSelection })
  const { close } = quick
  React.useEffect(() => {
    if (!negativeControl) return
    // Negative control: an unguarded `n` listener, and a host Escape handler that closes regardless of a pending create.
    const raw = (event: KeyboardEvent) => {
      if (event.key === 'n') latestOpen.current()
      if (event.key === 'Escape') close()
    }
    document.addEventListener('keydown', raw, true)
    return () => document.removeEventListener('keydown', raw, true)
  }, [negativeControl, close])
  const actions = React.useMemo<SidebarActions>(() => ({
    select: id => { const row = rows.find(candidate => candidate.id === id); if (row !== undefined) setSelected(row.ref) },
    note: quick.open,
    ...(shortcutOn ? { noteShortcut: 'N' } : {}),
  }), [quick.open, shortcutOn])
  const composing = quick.request?.target ?? selected
  const submit = async (text: string) => {
    const to = composing
    if (to === plannerTarget && holdPlanner.current) {
      holdPlanner.current = false
      const { promise, resolve } = Promise.withResolvers<void>()
      setTimeout(resolve, heldCreateMs)
      await promise
      throw new Error('synthetic create refusal')
    }
    await Promise.resolve()
    setNotes(current => ({ ...current, [to]: [publishedNote(text, to), ...(current[to] ?? [])] }))
  }
  return <NotesFrame scheme={scheme} width={width} title="Keyboard n popover">
    <ThemePortal>
      <NarrowLayoutContext.Provider value={touch}>
        <div ref={root} {...stylex.props(styles.stack)}>
          <p data-testid="popover-hint" {...stylex.props(frame.heading)}>
            {shortcutOn ? <>Press <kbd {...stylex.props(styles.kbd)}>N</kbd> to add a note to the selected agent.</> : 'The N shortcut is off. Use the row menu.'}
          </p>
          <div {...stylex.props(styles.workspace)}>
            <nav aria-label="Sidebar" {...stylex.props(styles.sidebar)}>
              <SidebarTree aria-label="Agents" items={items} expandedKeys={noneExpanded} onExpandedChange={() => undefined} onAction={key => setSelected(String(key))}
                renderRow={item => <div data-wf-agent-ref={item.row.ref}><MilestoneAgentRow row={item.row} active={item.id === selected} actions={actions} now={storyNow} /></div>} />
            </nav>
            <section aria-label="Agent pane" {...stylex.props(styles.pane)}>
              <header {...stylex.props(styles.paneHeader)}>
                <span data-testid="pane-title">{labels[selected]}</span>
                <span data-testid="pane-badge"><NoteBadge notes={notes[selected] ?? []} /></span>
              </header>
              <div role="log" aria-label="Transcript" tabIndex={0} data-testid="transcript" {...stylex.props(styles.transcript)}>{labels[selected]} is comparing the two diffs.</div>
            </section>
          </div>
          <section aria-label="Host widgets" {...stylex.props(frame.section)}>
            <TextField {...stylex.props(styles.field)}>
              <Label {...stylex.props(frame.heading)}>Filter agents</Label>
              <Input data-testid="popover-search" {...stylex.props(styles.input)} />
            </TextField>
            <div {...stylex.props(styles.field)}>
              <span id="notes-scratch-label" {...stylex.props(frame.heading)}>Scratch pad</span>
              <div contentEditable suppressContentEditableWarning aria-labelledby="notes-scratch-label" data-testid="popover-scratch" {...stylex.props(styles.input, styles.editable)} />
            </div>
            <div {...stylex.props(styles.field)}>
              <span id="notes-command-label" {...stylex.props(frame.heading)}>Command line</span>
              {/* A custom editor that owns its keys without being a native field or contenteditable. */}
              <div role="textbox" tabIndex={0} aria-labelledby="notes-command-label" data-testid="popover-command" onKeyDown={event => {
                if (event.key.length === 1 && !event.altKey && !event.ctrlKey && !event.metaKey) { event.preventDefault(); setCommandLine(line => line + event.key) }
              }} {...stylex.props(styles.input, styles.editable)}>{commandLine}</div>
            </div>
            {/* A closed host dialog stays in the DOM; it must not block the shortcut. */}
            <div role="dialog" aria-label="Closed host panel" hidden />
            <DialogTrigger>
              <Button variant="outline" size="md" data-testid="popover-shortcuts-trigger">Keyboard shortcuts</Button>
              <ModalOverlay isDismissable {...stylex.props(styles.overlay)}>
                <Modal {...stylex.props(styles.modal)}>
                  <Dialog aria-labelledby="notes-shortcuts-title" {...stylex.props(styles.modalDialog)}>
                    {({ close: dismiss }) => <>
                      <Heading id="notes-shortcuts-title" slot="title" {...stylex.props(styles.modalTitle)}>Keyboard shortcuts</Heading>
                      <dl {...stylex.props(styles.shortcuts)}>
                        <dt><kbd {...stylex.props(styles.kbd)}>N</kbd></dt><dd {...stylex.props(styles.shortcutText)}>Add a note to the selected agent (can be turned off)</dd>
                        <dt><kbd {...stylex.props(styles.kbd)}>Enter</kbd></dt><dd {...stylex.props(styles.shortcutText)}>Add the note</dd>
                        <dt><kbd {...stylex.props(styles.kbd)}>Shift+Enter</kbd></dt><dd {...stylex.props(styles.shortcutText)}>New line</dd>
                        <dt><kbd {...stylex.props(styles.kbd)}>Esc</kbd></dt><dd {...stylex.props(styles.shortcutText)}>Close and keep the draft</dd>
                      </dl>
                      <Button variant="outline" size="md" onPress={dismiss}>Close</Button>
                    </>}
                  </Dialog>
                </Modal>
              </ModalOverlay>
            </DialogTrigger>
          </section>
        </div>
        <NoteQuickAdd
          state={quick}
          targetLabel={labels[composing] ?? 'this agent'}
          shortcutEnabled={shortcutOn}
          onShortcutEnabledChange={setShortcutOn}
          draft={drafts[composing] ?? ''}
          onDraftChange={draft => setDrafts(current => ({ ...current, [composing]: draft }))}
          onSubmit={submit}
        />
      </NarrowLayoutContext.Provider>
    </ThemePortal>
  </NotesFrame>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/Explore/Notes/Keyboard n popover', component: KeyboardPopoverStory,
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Kit sidebar rows, local editable fields, dialogs, IME events and a held create pin the shortcut guard, the dismissal lock and per-target drafts; no scenario slice models subject notes.' }, layout: 'fullscreen' },
  args: notesFrameArgs,
  argTypes: notesFrameArgTypes('Add an unguarded n listener and a host Escape handler that ignores a pending create: the guard and dismissal-lock assertions must fail.'),
} satisfies Meta<typeof KeyboardPopoverStory>
export default meta
type Story = StoryObj<typeof meta>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStates:contract-fixture',
  'Dark1440:contract-fixture',
  'Light1440:contract-fixture',
  'Light440:contract-fixture',
  'TouchDark440:contract-fixture',
  'TouchLight440:contract-fixture',
]

const scopes = (canvasElement: HTMLElement) => {
  const canvas = within(canvasElement)
  const page = within(canvasElement.ownerDocument.body)
  const treeRow = (ref: string) => canvasElement.querySelector<HTMLElement>(`[data-sidebar-tree-key="${ref}"]`)!
  const noteDialog = () => page.queryByRole('dialog', { name: /^Add note to / })
  return {
    canvas, page, treeRow, noteDialog,
    menuTrigger: (ref: string) => treeRow(ref).querySelector<HTMLElement>('[data-agent-menu]')!,
    input: () => within(noteDialog()!).getByTestId('note-composer-input'),
    activeRow: () => canvasElement.ownerDocument.activeElement?.closest('[data-sidebar-tree-key]'),
  }
}

const assertStates: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const { canvas, page, treeRow, noteDialog, menuTrigger, input, activeRow } = scopes(canvasElement)
  const transcript = canvas.getByTestId('transcript')
  const count = () => within(canvas.getByTestId('pane-badge')).getByTestId('note-badge').getAttribute('data-note-count')
  const select = async (ref: string) => {
    await userEvent.click(treeRow(ref))
    await waitFor(() => expect(canvas.getByTestId('pane-title')).toHaveTextContent(labels[ref]!))
  }
  const openOn = async (ref: string) => {
    await select(ref)
    await userEvent.click(transcript)
    await userEvent.keyboard('n')
    const dialog = await page.findByRole('dialog', { name: `Add note to ${labels[ref]}` })
    await waitFor(() => expect(within(dialog).getByTestId('note-composer-input')).toHaveFocus())
    return dialog
  }
  const escapeTo = async (to: HTMLElement) => {
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(noteDialog()).toBeNull())
    await waitFor(() => expect(to).toHaveFocus())
  }
  await expectNoAudiencePresentation(canvasElement)
  await expect(page.queryAllByRole('switch')).toHaveLength(0)
  await expect(canvas.getByTestId('popover-hint')).toHaveTextContent('Press N to add a note')

  // Fields, editors, custom textboxes, open dialogs and IME composition keep their n.
  await userEvent.click(canvas.getByTestId('popover-search'))
  await userEvent.keyboard('n')
  await expect(canvas.getByTestId('popover-search')).toHaveValue('n')
  await expect(noteDialog()).toBeNull()
  await userEvent.click(canvas.getByTestId('popover-scratch'))
  await userEvent.keyboard('n')
  await expect(canvas.getByTestId('popover-scratch')).toHaveTextContent('n')
  await expect(noteDialog()).toBeNull()
  await userEvent.click(canvas.getByTestId('popover-command'))
  await userEvent.keyboard('n')
  await expect(canvas.getByTestId('popover-command')).toHaveTextContent('n')
  await expect(noteDialog()).toBeNull()
  transcript.dispatchEvent(new KeyboardEvent('keydown', { key: 'n', bubbles: true, cancelable: true, isComposing: true }))
  await expect(noteDialog()).toBeNull()
  const shortcutsTrigger = canvas.getByTestId('popover-shortcuts-trigger')
  await userEvent.click(shortcutsTrigger)
  await page.findByRole('dialog', { name: 'Keyboard shortcuts' })
  await userEvent.keyboard('n')
  await expect(noteDialog()).toBeNull()
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(page.queryByRole('dialog', { name: 'Keyboard shortcuts' })).toBeNull())
  await waitFor(() => expect(shortcutsTrigger).toHaveFocus())

  // The tree owns letters for typeahead: n moves focus to "Nightly checks" and opens nothing.
  treeRow(target).focus()
  await userEvent.keyboard('n')
  await waitFor(() => expect(activeRow()).toBe(treeRow(nightlyTarget)))
  await expect(noteDialog()).toBeNull()
  // An open row menu keeps n too.
  await userEvent.click(menuTrigger(target))
  await page.findByRole('menu', { name: 'Actions for Reviewer' })
  await userEvent.keyboard('n')
  await expect(noteDialog()).toBeNull()
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(page.queryByRole('menu')).toBeNull())

  // n opens on the selected row, named by title; Enter adds, announces and restores focus.
  const dialog = await openOn(target)
  await expect(within(dialog).getByTestId('note-composer-target')).toHaveTextContent('Target: Reviewer.')
  await expect(dialog).not.toHaveTextContent(target)
  await expect(within(dialog).getByTestId('note-composer-reach')).toHaveTextContent('Notes reach the agent as context.')
  await expect(within(dialog).getByRole('checkbox', { name: 'N opens note composer' })).toBeChecked()
  await expectNoAudiencePresentation(dialog)
  await userEvent.keyboard('Ship after review{Enter}')
  await waitFor(() => expect(noteDialog()).toBeNull())
  await expect(count()).toBe('3')
  await expect(page.getByTestId('note-quick-add-status')).toHaveTextContent('Note added to Reviewer.')
  await waitFor(() => expect(transcript).toHaveFocus())

  // While a create is in flight, Escape and outside presses leave the popover open; the refusal then lands in it.
  const plannerDialog = await openOn(plannerTarget)
  await userEvent.keyboard('Budget{Shift>}{Enter}{/Shift}')
  await userEvent.paste('line two')
  const multiline = 'Budget\nline two'
  await userEvent.keyboard('{Enter}')
  await expect(within(plannerDialog).getByTestId('note-composer-status')).toHaveTextContent('Adding note…')
  await userEvent.keyboard('{Escape}')
  await expect(noteDialog()).not.toBeNull()
  await userEvent.click(canvasElement.querySelector('h1')!)
  await expect(noteDialog()).not.toBeNull()
  await expect(await within(noteDialog()!).findByRole('alert', undefined, { timeout: heldCreateMs * 3 })).toHaveTextContent("Couldn't add the note. Your text is kept. synthetic create refusal")
  await expect(input()).toHaveValue(multiline)
  await expect(within(noteDialog()!).getByRole('button', { name: 'Retry' })).toBeVisible()
  // Settled, Escape closes again and keeps the draft for the next open.
  await userEvent.click(input())
  await escapeTo(transcript)
  await openOn(plannerTarget)
  await expect(input()).toHaveValue(multiline)
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(noteDialog()).toBeNull())
  await expect(page.getByTestId('note-quick-add-status')).toHaveTextContent('Note added to Planner.')
  await expect(count()).toBe('2')

  // The row menu's "Add note…" describes the shortcut on the selected row (it works from the conversation, not the
  // tree, where `n` is typeahead) and returns focus to the trigger.
  await userEvent.click(menuTrigger(plannerTarget))
  const item = await page.findByRole('menuitem', { name: 'Add note…' })
  await expect(item).toHaveAccessibleDescription('Press N in the conversation')
  await userEvent.click(item)
  await page.findByRole('dialog', { name: 'Add note to Planner' })
  await waitFor(() => expect(input()).toHaveFocus())
  await expect(input()).toHaveValue('')
  await escapeTo(menuTrigger(plannerTarget))

  // The checkbox turns n off: the shortcut and its hints go; the menu stays.
  await openOn(target)
  await userEvent.click(within(noteDialog()!).getByRole('checkbox', { name: 'N opens note composer' }))
  await expect(canvas.getByTestId('popover-hint')).toHaveTextContent('The N shortcut is off. Use the row menu.')
  await userEvent.click(input())
  await escapeTo(transcript)
  await userEvent.keyboard('n')
  await expect(noteDialog()).toBeNull()
  await userEvent.click(menuTrigger(target))
  const plain = await page.findByRole('menuitem', { name: /^Add note…/ })
  await expect(plain.textContent).toBe('Add note…')
  await userEvent.click(plain)
  await page.findByRole('dialog', { name: 'Add note to Reviewer' })
  await userEvent.click(within(noteDialog()!).getByRole('checkbox', { name: 'N opens note composer' }))
  await userEvent.click(input())
  await escapeTo(menuTrigger(target))
  await expect(canvas.getByTestId('popover-hint')).toHaveTextContent('Press N to add a note')

  await expectNoAudiencePresentation(canvasElement)
  await expect(greenFindings(canvasElement)).toEqual([])
}

const touchPlay: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const { page, noteDialog, menuTrigger, input } = scopes(canvasElement)
  await expect(page.queryAllByRole('switch')).toHaveLength(0)
  await userEvent.click(menuTrigger(target))
  await userEvent.click(await page.findByRole('menuitem', { name: /^Add note…/ }))
  await page.findByRole('dialog', { name: 'Add note to Reviewer' })
  const box = noteDialog()!.getBoundingClientRect()
  await expect(box.left).toBeGreaterThanOrEqual(0)
  await expect(box.right).toBeLessThanOrEqual(canvasElement.ownerDocument.documentElement.clientWidth)
  await userEvent.click(input())
  await userEvent.keyboard('From the phone')
  await userEvent.click(within(noteDialog()!).getByRole('button', { name: 'Add note' }))
  await waitFor(() => expect(noteDialog()).toBeNull())
  await expect(page.getByTestId('note-quick-add-status')).toHaveTextContent('Note added to Reviewer.')
  await expect(greenFindings(canvasElement)).toEqual([])
}

export const AllStates: Story = { play: assertStates }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play: assertStates }
export const Dark1440: Story = { args: { scheme: 'dark', width: 1440 }, play: assertStates }
export const Light1440: Story = { args: { scheme: 'light', width: 1440 }, play: assertStates }
export const TouchLight440: Story = { args: { scheme: 'light', width: 440, touch: true }, play: touchPlay }
export const TouchDark440: Story = { args: { scheme: 'dark', width: 440, touch: true }, play: touchPlay }

const styles = stylex.create({
  stack: { display: 'flex', flexDirection: 'column', gap: s.lg, minWidth: 0 },
  workspace: { display: 'flex', flexWrap: 'wrap', gap: s.lg, minWidth: 0, alignItems: 'flex-start' },
  sidebar: { flex: '0 1 280px', minWidth: 0, height: 200, display: 'flex', flexDirection: 'column' },
  pane: { flex: '1 1 320px', minWidth: 0, display: 'flex', flexDirection: 'column', borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md },
  paneHeader: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0, padding: s.sm, fontWeight: t.weightSemibold },
  transcript: { padding: s.md, minHeight: 96, color: text.fgMuted, fontSize: t.uiSize, lineHeight: t.uiLeading, borderRadius: r.sm, outlineStyle: 'none',
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset } },
  kbd: { fontFamily: t.fontMono, fontSize: t.denseSize, paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.keyboard },
  field: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0 },
  input: {
    minHeight: g.controlMd, boxSizing: 'border-box', paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong,
    borderRadius: r.control, backgroundColor: surface.controlFill, color: text.fg, fontFamily: t.fontSans, fontSize: t.metaSize,
    outlineStyle: 'none', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset },
  },
  editable: { display: 'flex', alignItems: 'center', fontFamily: t.fontMono, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  overlay: { position: 'fixed', inset: 0, backgroundColor: surface.scrim, display: 'flex', justifyContent: 'center', alignItems: 'center', zIndex: 30, padding: s.xl },
  modal: {
    width: `min(${g.modalMax}, calc(100vw - ${s.panel}))`, boxSizing: 'border-box', backgroundColor: surface.raised, color: text.fg,
    borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.lg, boxShadow: elevation.popover, fontFamily: t.fontSans,
  },
  modalDialog: { display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: s.md, padding: s.xl, outlineStyle: 'none' },
  modalTitle: { margin: s.zero, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightSemibold },
  shortcuts: { display: 'grid', gridTemplateColumns: 'max-content minmax(0, 1fr)', columnGap: s.md, rowGap: s.xs, margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading },
  shortcutText: { margin: s.zero, color: text.fgMuted, minWidth: 0 },
})
