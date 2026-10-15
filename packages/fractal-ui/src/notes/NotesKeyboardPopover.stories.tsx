import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button as AriaButton, Dialog, DialogTrigger, Heading, Input, Label, Menu, MenuItem, MenuTrigger, Modal, ModalOverlay, Popover, Switch, TextField } from 'react-aria-components'
import type { SubjectNote } from '@smalltalk/st3-client/schema'
import { NoteBadge, NoteComposer, defaultNoteAudience, shortcutContextBlocked, type NoteAudienceChoice, type NoteDraft } from './index'
import { agentNote, diffNote, plannerNote, plannerTarget, publishedNote, target } from './notes-fixtures'
import { NotesFrame, expectVisibleAgentMarker, notesFrameArgTypes, notesFrameArgs, notesStoryStyles as frame, type NotesFrameArgs } from './notes-story-frame'
import { Button, Tooltip } from '../assistant-ui/composition/Controls'
import { ThemePortal } from '../assistant-ui/taste/ThemePortal'
import { greenFindings } from '../assistant-ui/composition/no-green-assertion'
import { accentVars as accent, borderVars as border, elevationVars as elevation, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as text, typeVars as t } from '../assistant-ui/composition-tokens.stylex'

const targets = [target, plannerTarget] as const
type Target = typeof targets[number]
const emptyDraft: NoteDraft = { text: '', audience: defaultNoteAudience }
/** The planner refuses its first two creates, so failure, reopen and Retry stay reachable. */
const plannerRefusals = 2

/**
 * UX variant B — keyboard `n` popover. `n` (or Add note) opens a composer anchored to the selected
 * row and locked to that target while it is open. Escape closes without sending and keeps the
 * draft and audience per target; only Discard draft clears it. Focus returns to the trigger.
 * `n` never fires while a field, editor, menu, dialog or IME owns the keyboard, and the visible
 * switch turns the shortcut off.
 */
function KeyboardPopoverStory({ scheme, width, negativeControl }: NotesFrameArgs) {
  const [notes, setNotes] = React.useState<Readonly<Record<Target, readonly SubjectNote[]>>>({ [target]: [diffNote, agentNote], [plannerTarget]: [plannerNote] })
  const [selected, setSelected] = React.useState<Target>(target)
  // The target is captured when the composer opens; selection cannot retarget an open draft.
  const [composing, setComposing] = React.useState<Target | undefined>(undefined)
  const [drafts, setDrafts] = React.useState<Readonly<Partial<Record<Target, NoteDraft>>>>({})
  const [shortcutOn, setShortcutOn] = React.useState(true)
  const [commandLine, setCommandLine] = React.useState('')
  // Persistent announcement: the composer's own success status unmounts with the popover.
  const [announcement, setAnnouncement] = React.useState('')
  const refusals = React.useRef(plannerRefusals)
  const rowRefs = { [target]: React.useRef<HTMLButtonElement>(null), [plannerTarget]: React.useRef<HTMLButtonElement>(null) }
  const close = () => setComposing(undefined)
  // Same callback-ref subscription lifecycle and shared guard as the app's `/` search shortcut.
  const shortcut = React.useCallback((node: HTMLElement | null) => {
    if (node === null || !shortcutOn) return
    const listener = (event: KeyboardEvent) => {
      if (event.key !== 'n' || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return
      // Negative control: drop the context guard, so `n` typed into a field opens the composer.
      if (!negativeControl && shortcutContextBlocked(event)) return
      event.preventDefault()
      setComposing(selected)
    }
    // Capture, so a widget that swallows letters (menu typeahead) cannot hide a missing guard.
    document.addEventListener('keydown', listener, true)
    return () => document.removeEventListener('keydown', listener, true)
  }, [negativeControl, selected, shortcutOn])
  const submitTo = (to: Target) => async (text: string, audience: NoteAudienceChoice) => {
    await Promise.resolve()
    if (to === plannerTarget && refusals.current > 0) {
      refusals.current -= 1
      throw new Error('synthetic create refusal')
    }
    setNotes(current => ({ ...current, [to]: [publishedNote(text, audience, to), ...current[to]] }))
    setDrafts(current => ({ ...current, [to]: emptyDraft }))
    setAnnouncement(`Note added to ${to}.`)
    close()
  }
  return <NotesFrame scheme={scheme} width={width} title="Keyboard n popover">
    <ThemePortal>
      <div ref={shortcut} {...stylex.props(styles.stack)}>
        <div {...stylex.props(frame.row)}>
          <p data-testid="popover-hint" {...stylex.props(frame.heading, styles.grow)}>
            {shortcutOn ? <>Press <kbd {...stylex.props(styles.kbd)}>N</kbd> to add a note to the selected agent.</> : 'The N shortcut is off. Use Add note.'}
          </p>
          <Switch isSelected={shortcutOn} onChange={setShortcutOn} data-testid="popover-shortcut-switch" {...stylex.props(styles.switch)}>
            {({ isSelected, isFocusVisible }) => <>
              <span aria-hidden {...stylex.props(styles.track, isSelected && styles.trackOn, isFocusVisible && styles.trackFocus)}>
                <span {...stylex.props(styles.thumb, isSelected && styles.thumbOn)} />
              </span>
              N shortcut
            </>}
          </Switch>
        </div>
        <ul aria-label="Agents" {...stylex.props(styles.rows)}>
          {targets.map(ref => <li key={ref} {...stylex.props(styles.row, ref === selected && styles.selectedRow)}>
            <AriaButton ref={rowRefs[ref]} aria-pressed={ref === selected} aria-keyshortcuts={shortcutOn ? 'N' : undefined} data-testid={`row-${ref}`} onPress={() => setSelected(ref)} {...stylex.props(styles.rowButton)}>{ref}</AriaButton>
            <span data-testid={`badge-${ref}`}><NoteBadge notes={notes[ref]} /></span>
          </li>)}
        </ul>
        <div {...stylex.props(frame.row)}>
          <Tooltip label={shortcutOn ? 'Add a note to the selected agent (N)' : 'Add a note to the selected agent'}>
            <Button variant="outline" size="md" aria-keyshortcuts={shortcutOn ? 'N' : undefined} data-testid="popover-add-note" onPress={() => setComposing(selected)}>Add note</Button>
          </Tooltip>
          <MenuTrigger>
            <Button variant="outline" size="md" data-testid="popover-row-menu">Row actions</Button>
            <Popover placement="bottom start" {...stylex.props(styles.menuPopover)}>
              <Menu aria-label="Row actions" {...stylex.props(styles.menu)}>
                <MenuItem id="copy" {...stylex.props(styles.menuItem)}>Copy reference</MenuItem>
                <MenuItem id="next" {...stylex.props(styles.menuItem)}>Next needing attention</MenuItem>
              </Menu>
            </Popover>
          </MenuTrigger>
          <DialogTrigger>
            <Button variant="outline" size="md" data-testid="popover-shortcuts-trigger">Keyboard shortcuts</Button>
            <ModalOverlay isDismissable {...stylex.props(styles.overlay)}>
              <Modal {...stylex.props(styles.modal)}>
                <Dialog aria-labelledby="notes-shortcuts-title" {...stylex.props(styles.modalDialog)}>
                  {({ close: dismiss }) => <>
                    <Heading id="notes-shortcuts-title" slot="title" {...stylex.props(styles.modalTitle)}>Keyboard shortcuts</Heading>
                    <dl {...stylex.props(styles.shortcuts)}>
                      <dt><kbd {...stylex.props(styles.kbd)}>N</kbd></dt><dd {...stylex.props(styles.shortcutText)}>Add a note to the selected agent (switchable)</dd>
                      <dt><kbd {...stylex.props(styles.kbd)}>Tab</kbd></dt><dd {...stylex.props(styles.shortcutText)}>Move to the audience control; arrows or Space choose</dd>
                      <dt><kbd {...stylex.props(styles.kbd)}>Shift+Enter</kbd></dt><dd {...stylex.props(styles.shortcutText)}>New line</dd>
                      <dt><kbd {...stylex.props(styles.kbd)}>Esc</kbd></dt><dd {...stylex.props(styles.shortcutText)}>Close and keep the draft</dd>
                    </dl>
                    <Button variant="outline" size="md" onPress={dismiss}>Close</Button>
                  </>}
                </Dialog>
              </Modal>
            </ModalOverlay>
          </DialogTrigger>
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
        </section>
        <p role="status" aria-live="polite" aria-atomic="true" data-testid="popover-status" {...stylex.props(frame.heading)}>{announcement}</p>
      </div>
      <Popover triggerRef={rowRefs[composing ?? selected]} isOpen={composing !== undefined} onOpenChange={open => { if (!open) close() }} placement="bottom start" offset={6} {...stylex.props(styles.popover)}>
        {composing !== undefined && <Dialog aria-label={`Add note to ${composing}`} data-compose-target={composing} {...stylex.props(styles.dialog)}>
          <NoteComposer
            key={composing}
            target={composing}
            draft={drafts[composing] ?? emptyDraft}
            onDraftChange={draft => setDrafts(current => ({ ...current, [composing]: draft }))}
            onSubmit={submitTo(composing)}
            onClose={close}
            autoFocus
          />
        </Dialog>}
      </Popover>
    </ThemePortal>
  </NotesFrame>
}

const meta = {
  title: 'Fractal/Explore/Notes/Keyboard n popover', component: KeyboardPopoverStory, parameters: { layout: 'fullscreen' },
  args: notesFrameArgs,
  argTypes: notesFrameArgTypes('Drop the context guard: typing n into the filter opens the popover and the guard assertion must fail.'),
} satisfies Meta<typeof KeyboardPopoverStory>
export default meta
type Story = StoryObj<typeof meta>

const agentStatus = 'Audience: Shared with the agent (people can see it too).'

const assertStates: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const page = within(canvasElement.ownerDocument.body)
  const count = (ref: string) => within(canvas.getByTestId(`badge-${ref}`)).getByTestId('note-badge').getAttribute('data-note-count')
  const noteDialog = () => page.queryByRole('dialog', { name: /^Add note to / })
  const openOn = async (row: HTMLElement, ref: string) => {
    await userEvent.click(row)
    await userEvent.keyboard('n')
    const dialog = await page.findByRole('dialog', { name: `Add note to ${ref}` })
    await waitFor(() => expect(within(dialog).getByTestId('note-composer-input')).toHaveFocus())
    return dialog
  }
  const escapeTo = async (trigger: HTMLElement) => {
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(noteDialog()).toBeNull())
    await waitFor(() => expect(trigger).toHaveFocus())
  }
  const reviewer = canvas.getByTestId(`row-${target}`)
  const planner = canvas.getByTestId(`row-${plannerTarget}`)
  await expectVisibleAgentMarker(canvas.getByTestId(`badge-${target}`))
  await expect(canvas.getByTestId('popover-hint')).toHaveTextContent('Press N to add a note')

  // Fields, editors, custom textboxes, open menus, open dialogs and IME composition keep their n.
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
  await userEvent.click(reviewer)
  reviewer.dispatchEvent(new KeyboardEvent('keydown', { key: 'n', bubbles: true, cancelable: true, isComposing: true }))
  await expect(noteDialog()).toBeNull()
  const menuTrigger = canvas.getByTestId('popover-row-menu')
  await userEvent.click(menuTrigger)
  await page.findByRole('menu', { name: 'Row actions' })
  await userEvent.keyboard('n')
  await expect(noteDialog()).toBeNull()
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(page.queryByRole('menu')).toBeNull())
  await waitFor(() => expect(menuTrigger).toHaveFocus())
  const shortcutsTrigger = canvas.getByTestId('popover-shortcuts-trigger')
  await userEvent.click(shortcutsTrigger)
  await page.findByRole('dialog', { name: 'Keyboard shortcuts' })
  await userEvent.keyboard('n')
  await expect(noteDialog()).toBeNull()
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(page.queryByRole('dialog', { name: 'Keyboard shortcuts' })).toBeNull())
  await waitFor(() => expect(shortcutsTrigger).toHaveFocus())

  // The visible switch turns the shortcut off; Add note stays the pointer and keyboard entry.
  await userEvent.click(canvas.getByTestId('popover-shortcut-switch'))
  await expect(canvas.getByRole('switch', { name: 'N shortcut' })).not.toBeChecked()
  await expect(canvas.getByTestId('popover-hint')).toHaveTextContent('The N shortcut is off. Use Add note.')
  await userEvent.click(reviewer)
  await userEvent.keyboard('n')
  await expect(noteDialog()).toBeNull()
  await userEvent.click(canvas.getByTestId('popover-shortcut-switch'))
  await expect(canvas.getByRole('switch', { name: 'N shortcut' })).toBeChecked()

  // n opens on the selected row, announcing the target; Enter adds, closes and restores focus.
  const dialog = await openOn(reviewer, target)
  await expect(dialog).toHaveTextContent(`Target: ${target}.`)
  await expect(within(dialog).getByTestId('note-composer-input')).toHaveValue('')
  await expect(within(dialog).getByTestId('note-composer')).toHaveAttribute('data-note-audience', 'self')
  await userEvent.keyboard('Ship after review{Enter}')
  await waitFor(() => expect(noteDialog()).toBeNull())
  await expect(count(target)).toBe('3')
  await expect(canvas.getByTestId('popover-status')).toHaveTextContent(`Note added to ${target}.`)
  await waitFor(() => expect(reviewer).toHaveFocus())

  // Tab only moves to the labelled audience control; arrows choose and announce, Space keeps the choice.
  let plannerDialog = await openOn(planner, plannerTarget)
  await expect(plannerDialog).toHaveAttribute('data-compose-target', plannerTarget)
  const composer = () => within(noteDialog()!).getByTestId('note-composer')
  const input = () => within(noteDialog()!).getByTestId('note-composer-input')
  await userEvent.tab()
  await waitFor(() => expect(within(plannerDialog).getByTestId('note-audience-self')).toHaveFocus())
  await expect(composer()).toHaveAttribute('data-note-audience', 'self')
  await userEvent.keyboard('{ArrowRight}')
  await waitFor(() => expect(composer()).toHaveAttribute('data-note-audience', 'agent'))
  await expect(within(plannerDialog).getByTestId('note-composer-status')).toHaveTextContent(agentStatus)
  await userEvent.keyboard(' ')
  await expect(composer()).toHaveAttribute('data-note-audience', 'agent')

  // Multiline: Shift+Enter and pasted newlines survive; a refusal keeps text, audience and target.
  await userEvent.click(input())
  await userEvent.keyboard('Budget{Shift>}{Enter}{/Shift}')
  await userEvent.paste('line two\nline three')
  const multiline = 'Budget\nline two\nline three'
  await expect(input()).toHaveValue(multiline)
  await userEvent.keyboard('{Enter}')
  await expect(await within(plannerDialog).findByRole('alert')).toHaveTextContent("Couldn't add the note. Your text, audience and target are kept.")
  await expect(input()).toHaveValue(multiline)
  await expect(composer()).toHaveAttribute('data-note-audience', 'agent')
  await expect(within(plannerDialog).getByRole('button', { name: 'Retry' })).toBeVisible()

  // Escape closes without sending or discarding; the other target starts its own private draft.
  await escapeTo(planner)
  await expect(count(plannerTarget)).toBe('1')
  await openOn(reviewer, target)
  await expect(input()).toHaveValue('')
  await expect(composer()).toHaveAttribute('data-note-audience', 'self')
  await userEvent.keyboard('Reviewer draft')
  await escapeTo(reviewer)

  // Reopening the planner restores its draft; a second refusal offers Retry, which adds it.
  plannerDialog = await openOn(planner, plannerTarget)
  await expect(input()).toHaveValue(multiline)
  await expect(composer()).toHaveAttribute('data-note-audience', 'agent')
  await userEvent.keyboard('{Enter}')
  await within(plannerDialog).findByRole('alert')
  await userEvent.click(within(plannerDialog).getByRole('button', { name: 'Retry' }))
  await waitFor(() => expect(noteDialog()).toBeNull())
  await expect(count(plannerTarget)).toBe('2')
  await expect(canvas.getByTestId('popover-status')).toHaveTextContent(`Note added to ${plannerTarget}.`)
  await expectVisibleAgentMarker(canvas.getByTestId(`badge-${plannerTarget}`))
  await waitFor(() => expect(planner).toHaveFocus())

  // The reviewer draft survived the planner's whole round trip; only Discard draft clears it.
  const reviewerDialog = await openOn(reviewer, target)
  await expect(input()).toHaveValue('Reviewer draft')
  await userEvent.click(within(reviewerDialog).getByTestId('note-composer-discard'))
  await expect(input()).toHaveValue('')
  await expect(within(reviewerDialog).getByTestId('note-composer-status')).toHaveTextContent('Draft discarded.')
  await userEvent.click(input())
  await escapeTo(reviewer)
  await openOn(reviewer, target)
  await expect(input()).toHaveValue('')
  await escapeTo(reviewer)

  // A successful create resets the planner to an empty private draft.
  await openOn(planner, plannerTarget)
  await expect(input()).toHaveValue('')
  await expect(composer()).toHaveAttribute('data-note-audience', 'self')
  await escapeTo(planner)

  // Pointer entry: Add note opens on the selection and focus returns to Add note.
  const addNote = canvas.getByTestId('popover-add-note')
  await userEvent.click(addNote)
  await page.findByRole('dialog', { name: `Add note to ${plannerTarget}` })
  await waitFor(() => expect(input()).toHaveFocus())
  await escapeTo(addNote)

  await expect(greenFindings(canvasElement)).toEqual([])
}

export const AllStates: Story = { play: assertStates }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play: assertStates }
export const Dark1440: Story = { args: { scheme: 'dark', width: 1440 }, play: assertStates }
export const Light1440: Story = { args: { scheme: 'light', width: 1440 }, play: assertStates }

const styles = stylex.create({
  stack: { display: 'flex', flexDirection: 'column', gap: s.lg, minWidth: 0 },
  grow: { flex: '1 1 12rem', minWidth: 0 },
  kbd: { fontFamily: t.fontMono, fontSize: t.denseSize, paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.keyboard },
  switch: { display: 'inline-flex', alignItems: 'center', gap: s.sm, color: text.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer', flexShrink: 0 },
  track: {
    position: 'relative', display: 'inline-block', width: g.switchWidth, height: g.switchHeight, boxSizing: 'border-box', borderRadius: r.full,
    borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: surface.controlFill,
  },
  trackOn: { backgroundColor: accent.primary, borderColor: accent.primary },
  trackFocus: { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset },
  thumb: {
    position: 'absolute', insetBlock: g.hairline, insetInlineStart: g.hairline, aspectRatio: '1', borderRadius: r.full, backgroundColor: text.fgMuted,
    transitionProperty: 'transform', transitionDuration: '120ms', transitionTimingFunction: 'ease-out',
  },
  thumbOn: { backgroundColor: accent.onPrimary, transform: `translateX(calc(${g.switchWidth} - ${g.switchHeight}))` },
  field: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0 },
  input: {
    minHeight: g.controlMd, boxSizing: 'border-box', paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong,
    borderRadius: r.control, backgroundColor: surface.controlFill, color: text.fg, fontFamily: t.fontSans, fontSize: t.metaSize,
    outlineStyle: 'none', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset },
  },
  editable: { display: 'flex', alignItems: 'center', fontFamily: t.fontMono, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  rows: { listStyle: 'none', margin: s.zero, padding: s.zero, display: 'flex', flexDirection: 'column', gap: s.xs2, minWidth: 0 },
  row: { display: 'flex', alignItems: 'center', gap: s.md, minWidth: 0, minHeight: g.menuRow, paddingInline: s.sm, borderRadius: r.control },
  selectedRow: { backgroundColor: surface.rowActive },
  rowButton: {
    flex: '1 1 auto', minWidth: 0, textAlign: 'start', borderWidth: 0, backgroundColor: surface.transparent, color: text.fg, padding: s.zero,
    fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', cursor: 'pointer',
    outlineStyle: 'none', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset },
  },
  menuPopover: { backgroundColor: surface.raised, color: text.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, boxShadow: elevation.popover, minWidth: g.menuMin },
  menu: { display: 'flex', flexDirection: 'column', padding: s.xs2, outlineStyle: 'none', fontFamily: t.fontSans, fontSize: t.metaSize },
  menuItem: { paddingInline: s.sm, paddingBlock: s.xs, borderRadius: r.sm, cursor: 'pointer', outlineStyle: 'none', ':focus': { backgroundColor: surface.rowHover } },
  overlay: { position: 'fixed', inset: 0, backgroundColor: surface.scrim, display: 'flex', justifyContent: 'center', alignItems: 'center', zIndex: 30, padding: s.xl },
  modal: {
    width: `min(${g.modalMax}, calc(100vw - ${s.panel}))`, boxSizing: 'border-box', backgroundColor: surface.raised, color: text.fg,
    borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.lg, boxShadow: elevation.popover, fontFamily: t.fontSans,
  },
  modalDialog: { display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: s.md, padding: s.xl, outlineStyle: 'none' },
  modalTitle: { margin: s.zero, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightSemibold },
  shortcuts: { display: 'grid', gridTemplateColumns: 'max-content minmax(0, 1fr)', columnGap: s.md, rowGap: s.xs, margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading },
  shortcutText: { margin: s.zero, color: text.fgMuted, minWidth: 0 },
  popover: {
    width: `min(${g.modalMax}, calc(100vw - ${s.panel}))`, boxSizing: 'border-box', padding: s.xs,
    backgroundColor: surface.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.lg, boxShadow: elevation.popover,
  },
  dialog: { outlineStyle: 'none' },
})
