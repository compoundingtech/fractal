import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { useAtom } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import type { Key } from 'react-aria-components'
import type { SubjectNote } from '@fractal/client/schema'
import { greenFindings } from '../../../../packages/fractal-ui/src/assistant-ui/composition/no-green-assertion'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout'
import { MilestoneAgentRow } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/MilestoneAgentRow'
import { SidebarTree, type SidebarTreeItem } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarTree'
import type { SidebarActions, SidebarAgentRow as Row } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model'
import { ThemePortal } from '../../../../packages/fractal-ui/src/assistant-ui/taste/ThemePortal'
import { borderVars as border, radiusVars as r, spaceVars as s, textVars as text, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex'
import { NotesFrame, notesFrameArgs, notesFrameArgTypes, expectNoAudiencePresentation, type NotesFrameArgs } from '../../../../packages/fractal-ui/src/notes/notes-story-frame'
import { target, plannerTarget, retireNote, publishedNote, viewer } from '../../../../packages/fractal-ui/src/notes/notes-fixtures'
import { fixtureSource } from '../data/fixtureSource'
import { fixtureProjections } from '../fixtures/projections'
import { DataSourceProvider } from '../data/react'
import { observed, unavailable, waiting, type DataSource, type Feed } from '../data/source'
import { AgentNotes, noteShortcutPreference, useAgentNoteQuickAdd } from './AgentNotes'

const storyNow = Date.UTC(2026, 9, 14, 16, 30, 0)
const nightlyTarget = 'agent/example/nightly'
const reviewerRow: Row = {
  ref: target, id: 'reviewer', title: 'Reviewer', status: 'idle', statusLabel: 'Idle', host: 'build-1', freshness: 'live',
  usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Unknown' }, children: [],
}
/** "Nightly checks" starts with `n`: typeahead from the tree must land there instead of opening a note. */
const rows: readonly Row[] = [reviewerRow, { ...reviewerRow, ref: plannerTarget, id: 'planner', title: 'Planner' }, { ...reviewerRow, ref: nightlyTarget, id: 'nightly', title: 'Nightly checks' }]
interface AgentItem extends SidebarTreeItem<AgentItem> { readonly row: Row }
const items: readonly AgentItem[] = rows.map((row): AgentItem => ({ id: row.ref, depth: 0, textValue: row.title, row, disclosurePlacement: 'trailing' }))
const noneExpanded: ReadonlySet<Key> = new Set()
const labelFor = (ref: string) => rows.find(row => row.ref === ref)?.title

interface WebNotesArgs extends NotesFrameArgs {
  /** Phone layout: touch rows, no keyboard shortcut path. */
  readonly touch?: boolean
}

interface Fixture {
  readonly source: DataSource
  readonly registry: AtomRegistry.AtomRegistry
  /** Negative control only: a second registry that does not share drafts. */
  readonly splitRegistry: AtomRegistry.AtomRegistry
}

const makeFixture = (): Fixture => {
  const registry = AtomRegistry.make()
  const splitRegistry = AtomRegistry.make()
  const notes = Atom.make<Feed<readonly SubjectNote[]>>(observed({ value: [retireNote] })).pipe(Atom.keepAlive)
  let refusals = 1
  const source = { ...fixtureSource({ world: fixtureProjections }), notes, notesActor: Atom.make<string | undefined>(viewer), noteActions: {
    create: async (to: string, text: string) => {
      if (refusals-- > 0) throw new Error('Synthetic gateway refusal.')
      const previous = registry.get(notes)
      registry.set(notes, observed({ value: [publishedNote(text, to), ...(previous._tag === 'Observed' ? previous.value : [])] }))
    },
    archive: async () => undefined,
  } }
  return { source, registry, splitRegistry }
}

/** The production wiring at story scale: the kit sidebar rows carry the menu action, the pane carries the inline entry. */
function Workspace({ fixture, negativeControl }: { readonly fixture: Fixture; readonly negativeControl: boolean }) {
  const [selected, select] = React.useState<string>(target)
  const [, setShortcut] = useAtom(noteShortcutPreference)
  // The preference persists; every run starts from the default.
  React.useLayoutEffect(() => setShortcut(true), [setShortcut])
  const { noteActions, notePopover } = useAgentNoteQuickAdd({ current: selected, labelFor })
  const actions = React.useMemo<SidebarActions>(() => ({
    select: id => { const row = rows.find(candidate => candidate.id === id); if (row !== undefined) select(row.ref) },
    ...noteActions,
  }), [noteActions])
  return <div {...stylex.props(styles.workspace)}>
    <nav aria-label="Sidebar" {...stylex.props(styles.sidebar)}>
      <SidebarTree aria-label="Agents" items={items} expandedKeys={noneExpanded} onExpandedChange={() => undefined} onAction={key => select(String(key))}
        renderRow={item => <div data-wf-agent-ref={item.row.ref}><MilestoneAgentRow row={item.row} active={item.id === selected} actions={actions} now={storyNow} /></div>} />
    </nav>
    <section aria-label="Agent pane" {...stylex.props(styles.pane)}>
      <AgentNotes agentRef={selected} targetLabel={labelFor(selected) ?? ''} />
      <div role="log" aria-label="Transcript" tabIndex={0} data-testid="transcript" {...stylex.props(styles.transcript)}>
        {labelFor(selected)} is comparing the two diffs.
      </div>
    </section>
    {/* Negative control: a separate registry breaks the shared per-target draft. */}
    {negativeControl ? <DataSourceProvider source={fixture.source} registry={fixture.splitRegistry}>{notePopover}</DataSourceProvider> : notePopover}
  </div>
}

function WebNotes({ scheme, width, negativeControl, touch = false }: WebNotesArgs) {
  const [fixture] = React.useState(makeFixture)
  return <NotesFrame scheme={scheme} width={width} title="Web notes: agent pane and sidebar row">
    <ThemePortal>
      <NarrowLayoutContext.Provider value={touch}>
        <DataSourceProvider source={fixture.source} registry={fixture.registry}>
          <Workspace fixture={fixture} negativeControl={negativeControl} />
        </DataSourceProvider>
      </NarrowLayoutContext.Provider>
    </ThemePortal>
  </NotesFrame>
}

const longWord = `Reconcile${'theretainedwindowagainstthegateway'.repeat(5)}`
const longTitle = `Reviewer${'withaverylongsingleworldtitle'.repeat(4)}`
interface FeedCase {
  readonly id: string
  readonly label: string
  readonly feed: Feed<readonly SubjectNote[]>
  readonly actor?: string | undefined
  readonly targetLabel?: string
}
const feedCases = (negativeControl: boolean): readonly FeedCase[] => [
  { id: 'waiting', label: 'Loading', feed: waiting, actor: viewer },
  { id: 'unavailable', label: 'Gateway without notes', feed: unavailable({ reason: 'unsupported', detail: 'This gateway does not serve notes.' }), actor: viewer },
  // Negative control: the stale window is published as live.
  { id: 'stale', label: 'Out of date', feed: observed({ value: [retireNote], freshness: negativeControl ? 'live' : 'stale' }), actor: viewer },
  { id: 'partial', label: 'Bounded window', feed: { ...observed({ value: [retireNote] }), coverage: { _tag: 'Partial' } }, actor: viewer },
  { id: 'no-actor', label: 'Identity pending', feed: observed({ value: [retireNote] }), actor: undefined },
  { id: 'long', label: 'Long single words', feed: observed({ value: [publishedNote(longWord, target)] }), actor: viewer, targetLabel: longTitle },
]

function FeedCasePane({ feedCase }: { readonly feedCase: FeedCase }) {
  const [fixture] = React.useState(() => {
    const registry = AtomRegistry.make()
    const source = { ...fixtureSource({ world: fixtureProjections }), notes: Atom.make(feedCase.feed), notesActor: Atom.make<string | undefined>(feedCase.actor),
      noteActions: { create: async () => undefined, archive: async () => undefined } }
    return { registry, source }
  })
  return <section aria-label={feedCase.label} data-feed-case={feedCase.id} {...stylex.props(styles.feedCase)}>
    <h2 {...stylex.props(styles.heading)}>{feedCase.label}</h2>
    <DataSourceProvider source={fixture.source} registry={fixture.registry}>
      <AgentNotes agentRef={target} targetLabel={feedCase.targetLabel ?? 'Reviewer'} />
    </DataSourceProvider>
  </section>
}

function FeedStates({ scheme, width, negativeControl }: WebNotesArgs) {
  return <NotesFrame scheme={scheme} width={width} title="Web notes: feed states">
    <ThemePortal>
      <div {...stylex.props(styles.feedCases)}>
        {feedCases(negativeControl).map(feedCase => <FeedCasePane key={feedCase.id} feedCase={feedCase} />)}
      </div>
    </ThemePortal>
  </NotesFrame>
}

const meta = {
  title: 'Fractal/Plugins/Notes/Web entry points', component: WebNotes,
  excludeStories: ['scenarioExemptions'],
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Real web note adapters and kit sidebar rows share the data-source registry over synthetic note publication and a deterministic refused create.' } },
  args: notesFrameArgs,
  argTypes: notesFrameArgTypes('Entry points: split the quick-add and pane registries, so the shared-draft assertion fails. Feed states: publish the stale window as live, so the out-of-date assertion fails.'),
} satisfies Meta<typeof WebNotes>
export default meta
type Story = StoryObj<typeof meta>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStates:contract-fixture',
  'Dark1440:contract-fixture',
  'FeedStatesDark1440:contract-fixture',
  'FeedStatesLight1440:contract-fixture',
  'FeedStatesLight440:contract-fixture',
  'Light1440:contract-fixture',
  'Light440:contract-fixture',
  'TouchDark440:contract-fixture',
  'TouchLight440:contract-fixture',
]

const scopes = (canvasElement: HTMLElement) => {
  const canvas = within(canvasElement)
  const page = within(canvasElement.ownerDocument.body)
  const notes = () => within(canvas.getByTestId('agent-notes'))
  const treeRow = (ref: string) => canvasElement.querySelector<HTMLElement>(`[data-sidebar-tree-key="${ref}"]`)!
  return {
    canvas, page, notes, treeRow,
    addNote: () => notes().getByTestId('agent-notes-add'),
    inline: () => notes().getByTestId('note-composer-input') as HTMLTextAreaElement,
    dialog: (label = 'Reviewer') => page.getByRole('dialog', { name: `Add note to ${label}` }),
    menuTrigger: (ref: string) => treeRow(ref).querySelector<HTMLElement>('[data-agent-menu]')!,
  }
}

const play: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const { canvas, page, notes, treeRow, addNote, inline, dialog, menuTrigger } = scopes(canvasElement)
  const popoverInput = (label?: string) => within(dialog(label)).getByTestId('note-composer-input')

  // At rest the pane spends one line: the disclosure and a quiet Add note. No composer, no switch.
  await expect(page.queryAllByRole('switch')).toHaveLength(0)
  await expect(notes().queryByTestId('note-composer')).toBeNull()
  const summary = notes().getByTestId('agent-notes-summary')
  await expect(within(summary).getByTestId('note-badge')).toHaveAttribute('data-note-badge-static')
  await expect(summary.querySelectorAll('[tabindex], [role="img"]')).toHaveLength(0)

  // Add note expands the shared composer; the pane names the target, so the composer does not repeat it.
  await userEvent.click(addNote())
  await waitFor(() => expect(inline()).toHaveFocus())
  await expect(notes().getByTestId('note-composer')).toHaveAttribute('data-surface', 'bare')
  await expect(notes().queryByTestId('note-composer-target')).toBeNull()
  await userEvent.keyboard('Keep this draft')
  await userEvent.keyboard('n')
  await expect(page.queryByRole('dialog')).toBeNull()
  await expect(inline()).toHaveValue('Keep this draftn')
  // Escape collapses back to Add note and keeps the draft.
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(addNote()).toHaveFocus())
  await expect(notes().queryByTestId('note-composer')).toBeNull()

  // From the transcript, `n` opens the quick-add on the selected row with the same draft; IME composition never does.
  const transcript = canvas.getByTestId('transcript')
  transcript.dispatchEvent(new KeyboardEvent('keydown', { key: 'n', bubbles: true, cancelable: true, isComposing: true }))
  await expect(page.queryByRole('dialog')).toBeNull()
  await userEvent.click(transcript)
  await userEvent.keyboard('n')
  await waitFor(() => expect(popoverInput()).toHaveFocus())
  await expect(popoverInput()).toHaveValue('Keep this draftn')
  await expect(within(dialog()).getByRole('checkbox', { name: 'N opens note composer' })).toBeChecked()
  // Away from the pane, the popover names its target by title.
  await expect(within(dialog()).getByTestId('note-composer-target')).toHaveTextContent('Target: Reviewer.')
  // Anchored under the selected row's menu trigger.
  await expect(dialog().getBoundingClientRect().top).toBeGreaterThanOrEqual(menuTrigger(target).getBoundingClientRect().bottom - 1)

  // A refusal keeps the text; Retry adds, announces the target and returns focus to the transcript.
  await userEvent.click(within(dialog()).getByRole('button', { name: 'Add note' }))
  await expect(await within(dialog()).findByRole('alert')).toHaveTextContent("Couldn't add the note. Your text is kept. Synthetic gateway refusal.")
  await expect(popoverInput()).toHaveValue('Keep this draftn')
  await userEvent.click(within(dialog()).getByRole('button', { name: 'Retry' }))
  await waitFor(() => expect(page.queryByRole('dialog')).toBeNull())
  await expect(page.getByTestId('note-quick-add-status')).toHaveTextContent('Note added to Reviewer.')
  await waitFor(() => expect(transcript).toHaveFocus())
  // One draft per target: the pane's draft was the one sent.
  await userEvent.click(addNote())
  await waitFor(() => expect(inline()).toHaveValue(''))
  await userEvent.keyboard('{Escape}')
  // Add note opened the disclosure; the summary toggles it.
  const disclosure = notes().getByTestId('agent-notes-disclosure')
  await expect(disclosure).toHaveAttribute('open')
  await userEvent.click(summary)
  await waitFor(() => expect(disclosure).not.toHaveAttribute('open'))
  await userEvent.click(summary)
  await waitFor(() => expect(disclosure).toHaveAttribute('open'))
  await expect(notes().getByTestId('note-list')).toHaveTextContent('Keep this draftn')
  await expect(notes().getByTestId('note-session')).toHaveTextContent('This session')

  // In the tree `n` is typeahead: focus moves to "Nightly checks" and nothing opens.
  treeRow(target).focus()
  await userEvent.keyboard('n')
  await waitFor(() => expect(canvasElement.ownerDocument.activeElement?.closest('[data-sidebar-tree-key]')).toBe(treeRow(nightlyTarget)))
  await expect(page.queryByRole('dialog')).toBeNull()

  // The selected row's menu offers Add note… with its shortcut; focus returns to the trigger on close.
  await userEvent.click(menuTrigger(target))
  const selectedItem = await page.findByRole('menuitem', { name: /^Add note…/ })
  await expect(selectedItem.textContent).not.toBe('Add note…')
  await userEvent.click(selectedItem)
  await waitFor(() => expect(popoverInput()).toHaveFocus())
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(page.queryByRole('dialog')).toBeNull())
  await waitFor(() => expect(menuTrigger(target)).toHaveFocus())
  // Another row's menu targets that row, without the selection's shortcut hint.
  await userEvent.click(menuTrigger(plannerTarget))
  const otherItem = await page.findByRole('menuitem', { name: /^Add note…/ })
  await expect(otherItem.textContent).toBe('Add note…')
  await userEvent.click(otherItem)
  await waitFor(() => expect(popoverInput('Planner')).toHaveFocus())
  await expect(popoverInput('Planner')).toHaveValue('')
  // Turning the shortcut off releases `n` and drops the hint.
  await userEvent.click(within(dialog('Planner')).getByRole('checkbox', { name: 'N opens note composer' }))
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(page.queryByRole('dialog')).toBeNull())
  await userEvent.click(transcript)
  await userEvent.keyboard('n')
  await expect(page.queryByRole('dialog')).toBeNull()
  await userEvent.click(menuTrigger(target))
  await expect((await page.findByRole('menuitem', { name: /^Add note…/ })).textContent).toBe('Add note…')
  await userEvent.keyboard('{Escape}')

  await expectNoAudiencePresentation(canvasElement)
  await expect(greenFindings(canvasElement)).toEqual([])
}

const touchPlay: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const { page, addNote, inline, dialog, menuTrigger } = scopes(canvasElement)
  await expect(page.queryAllByRole('switch')).toHaveLength(0)
  await userEvent.click(addNote())
  await waitFor(() => expect(inline()).toHaveFocus())
  await userEvent.keyboard('From the phone')
  // The row menu reaches the same draft for the same target.
  await userEvent.click(menuTrigger(target))
  await userEvent.click(await page.findByRole('menuitem', { name: /^Add note…/ }))
  const input = await within(await waitFor(() => dialog())).findByTestId('note-composer-input')
  await expect(input).toHaveValue('From the phone')
  const box = dialog().getBoundingClientRect()
  await expect(box.left).toBeGreaterThanOrEqual(0)
  await expect(box.right).toBeLessThanOrEqual(canvasElement.ownerDocument.documentElement.clientWidth)
  await userEvent.click(within(dialog()).getByRole('button', { name: 'Add note' }))
  await within(dialog()).findByRole('alert')
  await userEvent.click(within(dialog()).getByRole('button', { name: 'Retry' }))
  await waitFor(() => expect(page.queryByRole('dialog')).toBeNull())
  await expect(page.getByTestId('note-quick-add-status')).toHaveTextContent('Note added to Reviewer.')
  await waitFor(() => expect(inline()).toHaveValue(''))
  await expect(greenFindings(canvasElement)).toEqual([])
}

const feedPlay: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const pane = (id: string) => within(canvasElement.querySelector<HTMLElement>(`[data-feed-case="${id}"]`)!)
  const openComposer = async (id: string) => {
    await userEvent.click(pane(id).getByTestId('agent-notes-add'))
    return pane(id).getByTestId('note-composer')
  }

  await expect(pane('waiting').getByTestId('agent-notes-feed-status')).toHaveTextContent('Loading notes…')
  const waitingComposer = await openComposer('waiting')
  await expect(waitingComposer).toHaveTextContent('Wait for the notes connection before adding a note.')
  await expect(within(waitingComposer).getByTestId('note-composer-submit')).toBeDisabled()

  await expect(pane('unavailable').getByTestId('agent-notes-feed-status')).toHaveTextContent('Notes are not available from this gateway.')
  await expect(pane('unavailable').queryByTestId('agent-notes-add')).toBeNull()

  await expect(pane('stale').getByTestId('agent-notes-stale')).toHaveTextContent('Notes are out of date. Waiting for the connection.')
  await expect(await openComposer('stale')).toHaveTextContent('Wait for the notes connection before adding a note.')
  await expect(pane('stale').queryByRole('button', { name: 'Archive' })).toBeNull()

  await expect(pane('partial').getByTestId('agent-notes-partial')).toHaveTextContent('Showing notes in the current window.')

  await expect(await openComposer('no-actor')).toHaveTextContent('Waiting for your signed-in identity.')

  const long = canvasElement.querySelector<HTMLElement>('[data-feed-case="long"]')!
  await userEvent.click(pane('long').getByTestId('agent-notes-summary'))
  await expect(pane('long').getByTestId('note-list')).toHaveTextContent(longWord)
  await expect(long.scrollWidth).toBeLessThanOrEqual(long.clientWidth)

  await expect(canvasElement.textContent).not.toMatch(/unknown/i)
  await expectNoAudiencePresentation(canvasElement)
  await expect(greenFindings(canvasElement)).toEqual([])
}

export const AllStates: Story = { play }
export const Dark1440: Story = { args: { scheme: 'dark', width: 1440 }, play }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play }
export const Light1440: Story = { args: { scheme: 'light', width: 1440 }, play }
export const TouchLight440: Story = { args: { scheme: 'light', width: 440, touch: true }, play: touchPlay }
export const TouchDark440: Story = { args: { scheme: 'dark', width: 440, touch: true }, play: touchPlay }
export const FeedStatesLight440: Story = { args: { scheme: 'light', width: 440 }, render: args => <FeedStates {...args} />, play: feedPlay }
export const FeedStatesDark1440: Story = { args: { scheme: 'dark', width: 1440 }, render: args => <FeedStates {...args} />, play: feedPlay }
export const FeedStatesLight1440: Story = { args: { scheme: 'light', width: 1440 }, render: args => <FeedStates {...args} />, play: feedPlay }

const styles = stylex.create({
  workspace: { display: 'flex', flexWrap: 'wrap', gap: s.lg, minWidth: 0, alignItems: 'flex-start' },
  sidebar: { flex: '0 1 280px', minWidth: 0, height: 200, display: 'flex', flexDirection: 'column' },
  pane: { flex: '1 1 320px', minWidth: 0, display: 'flex', flexDirection: 'column', gap: s.sm, borderWidth: 1, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md },
  transcript: { padding: s.md, color: text.fgMuted, fontSize: t.uiSize, lineHeight: t.uiLeading, minHeight: 96 },
  feedCases: { display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(320px, 1fr))', gap: s.lg, minWidth: 0 },
  feedCase: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0, borderWidth: 1, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, overflow: 'hidden' },
  heading: { margin: s.zero, paddingInline: s.md, paddingBlockStart: s.sm, fontSize: t.metaSize, lineHeight: t.metaLeading, color: text.fgMuted },
})
