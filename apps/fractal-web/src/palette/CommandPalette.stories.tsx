import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import { currentSpace, emptyWorkspace, panesOf, transition, type Command, type WorkspaceConfig, type WorkspaceState } from '@fractal/ui/workspace'
import { memoryRecentStore } from '@fractal/ui/workspace/palette'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { CommandPalette, PaletteButton, usePaletteOpen, type SaidSearch } from './CommandPalette.tsx'
import { useLiveHostCommands, useLivePalette } from './live.ts'
import type { HostCommand, PaletteWorld } from './items.ts'

const world: PaletteWorld = {
  alerts: [
    { ref: 'attention/release-plan', title: 'Approve the release plan', kind: 'decision', status: { tone: 'needs-you', label: 'Waiting for you' } },
    { ref: 'attention/ci-red', title: 'CI failed on main', kind: 'failure', status: { tone: 'failed', label: 'Failed' } },
  ],
  agents: [
    { ref: 'agent/example/atlas/builder', name: 'Atlas Builder', detail: 'claude · juniper', status: { tone: 'working', label: 'Working' }, terminal: true },
    { ref: 'agent/example/harbor/keeper', name: 'Harbor Keeper', detail: 'codex · juniper', status: { tone: 'needs-you', label: 'Waiting for you' }, terminal: true },
    { ref: 'agent/example/new/scout', name: 'Newsroom Scout', detail: 'omp · dev3', status: { tone: 'done', label: 'Ended' } },
  ],
  missions: [
    { ref: 'mission/release-3-2', title: 'Release 3.2', word: 'running', status: { tone: 'working', label: 'Working' } },
    { ref: 'mission/palette', title: 'Web command palette', word: 'waiting', status: { tone: 'needs-you', label: 'Waiting for you' } },
  ],
  machines: [{ id: 'juniper', connected: true }, { id: 'dev3', connected: false }],
  resources: [
    { ref: 'resource/github/compoundingtech/fractal/pull-request/5120', title: 'Palette at TUI parity', kind: 'pull request' },
    { ref: 'resource/github/compoundingtech/fractal/issue/88', title: 'Spaces lose focus on rename', kind: 'issue' },
  ],
}

const said: SaidSearch = async query => query.toLowerCase().startsWith('harb')
  ? { items: [{ agent_id: 'agent/example/harbor/keeper', conversation_id: 'c1', excerpt: 'The harbor keys rotate every Monday at nine.', timestamp: '2026-11-08T09:30:00Z' }], refreshing: true }
  : { items: [] }

let id = 0
const config: WorkspaceConfig = { newId: () => `s${id++}` }
const initial = (): WorkspaceState => [
  { _tag: 'Open', ref: 'agent:agent/example/atlas/builder' } as const,
  { _tag: 'Open', ref: 'list:home' } as const,
].reduce((state, command) => transition(state, command, config), emptyWorkspace(config, { name: 'Main' }))

interface HarnessProps {
  readonly scheme: 'dark' | 'light'
  readonly phone?: boolean
  readonly recent?: readonly string[]
  /** How many agents the roster holds beyond the fixed three. */
  readonly extraAgents?: number
}

/** A palette over a real workspace transition; the readout below shows what it dispatched. */
const noRecent: readonly string[] = []
const Harness = ({ scheme, phone = false, recent = noRecent, extraAgents = 0 }: HarnessProps) => {
  const [state, setState] = React.useState(initial)
  const [ran, setRan] = React.useState<string | undefined>(undefined)
  const [recentStore] = React.useState(() => memoryRecentStore(recent))
  const roster = React.useMemo((): PaletteWorld => extraAgents === 0 ? world : {
    ...world,
    agents: [...world.agents, ...Array.from({ length: extraAgents }, (_, index) => ({ ref: `agent/example/fleet/worker-${index + 1}`, name: `Worker ${index + 1}`, detail: 'omp · dev5' }))],
  }, [extraAgents])
  const palette = usePaletteOpen()
  const dispatch = React.useCallback((command: Command) => setState(current => transition(current, command, config)), [])
  const commands = React.useMemo((): readonly HostCommand[] => [
    { id: 'new-mission', section: 'start', label: 'New mission', detail: 'describe it; a planner drafts it for you to approve', search: 'new mission launch', run: () => setRan('new-mission') },
    { id: 'new-terminal', section: 'start', label: 'New terminal', detail: 'a shell in a new tab', search: 'new terminal shell', run: () => setRan('new-terminal') },
    { id: 'theme', section: 'commands', label: 'Switch to light theme', search: 'theme light dark appearance', icon: 'swap', run: () => setRan('theme') },
    { id: 'settings', section: 'commands', label: 'Settings', search: 'settings preferences', icon: 'gear', run: () => setRan('settings') },
    { id: 'sidebar-agents', section: 'sidebar', label: 'Sidebar: agents', detail: 'mode', search: 'sidebar agents mode', icon: 'panel-left', run: () => setRan('sidebar-agents') },
  ], [])
  const space = currentSpace(state)
  return <main data-scheme={scheme} {...stylex.props(styles.root, phone && styles.phone, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
    <header {...stylex.props(styles.bar)}>
      {phone ? <PaletteButton onPress={palette.open} /> : <button type="button" onClick={palette.open}>Open palette</button>}
    </header>
    <section aria-label="Workspace" data-testid="readout" {...stylex.props(styles.readout)}>
      <div data-testid="space">{space.name} ({state.spaces.length} spaces)</div>
      {panesOf(space.layout).map((pane, index) => <div key={pane.id} data-testid={`pane-${index}`}>{pane.tabs.map(tab => `${tab.ref}${tab.id === pane.active ? '*' : ''}`).join(' | ')}</div>)}
      {ran !== undefined && <div data-testid="ran">{ran}</div>}
    </section>
    <CommandPalette {...palette} state={state} dispatch={dispatch} world={roster} commands={commands} search={said} recentStore={recentStore} phone={phone} />
  </main>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Workspace/Command palette',
  component: Harness,
  parameters: {
    layout: 'fullscreen',
    scenario: { _tag: 'contract-fixture', reason: 'A fixed roster of agents, missions, alerts, machines and resources over a real workspace transition pins the palette’s sections, ranking, prefix scopes, open targets, naming mode and the phone sheet in both themes.' },
    docs: { description: { component: 'Ctrl+T (Mac) or Cmd/Ctrl+K opens one input over sectioned rows at TUI parity. Enter opens here, Cmd/Ctrl+Enter in a tab, Alt+Enter right, Alt+Shift+Enter below; > @ # ! / scope; Alt+1–8 jump.' } },
  },
  args: { scheme: 'dark' },
} satisfies Meta<typeof Harness>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'CappedSections:contract-fixture',
  'Empty:contract-fixture',
  'EmptyLight:contract-fixture',
  'LiveFixtures:contract-fixture',
  'LongSection:contract-fixture',
  'Naming:contract-fixture',
  'NoMatches:contract-fixture',
  'Phone:contract-fixture',
  'PhoneLight:contract-fixture',
  'SaidInConversations:contract-fixture',
  'Scoping:contract-fixture',
  'Targets:contract-fixture',
  'Typing:contract-fixture',
]
type Story = StoryObj<typeof meta>

const body = () => within(document.body)
const openPalette = async (canvasElement: HTMLElement) => {
  await userEvent.click(within(canvasElement).getByRole('button', { name: /open palette|search/i }))
  const dialog = await body().findByRole('dialog', { name: 'Command palette' })
  const input = within(dialog).getByRole('combobox')
  await waitFor(() => expect(input).toHaveFocus())
  return { dialog, input }
}
const sectionsOf = (dialog: HTMLElement) => within(dialog).getAllByRole('group').map(group => group.getAttribute('aria-label'))
const selected = (dialog: HTMLElement) => dialog.querySelector('[role="option"][aria-selected="true"]')?.getAttribute('data-palette-row')
const closed = () => waitFor(() => expect(body().queryByRole('dialog', { name: 'Command palette' })).toBeNull())

/** Empty: Recent first, then the TUI's sections in order; Escape closes and focus returns. */
export const Empty: Story = {
  args: { recent: ['mission:mission/palette', 'settings'] },
  play: async ({ canvasElement }) => {
    const { dialog } = await openPalette(canvasElement)
    await expect(sectionsOf(dialog)).toEqual(['Recent', 'Alerts', 'Agents', 'Missions', 'Fleet', 'Spaces', 'Start', 'Sidebar', 'Resources', 'Commands'])
    await expect(selected(dialog)).toBe('recent:mission:mission/palette')
    await expect(dialog.querySelector('[data-palette-hints]')).toHaveTextContent(/here.*tab.*right.*below/u)
    await userEvent.keyboard('{Escape}')
    await closed()
    await waitFor(() => expect(within(canvasElement).getByRole('button', { name: 'Open palette' })).toHaveFocus())
  },
}

export const EmptyLight: Story = { ...Empty, args: { ...Empty.args, scheme: 'light' } }

/** Typing ranks agents first, then missions; a start command typed as it begins comes first. */
export const Typing: Story = {
  play: async ({ canvasElement }) => {
    const { dialog, input } = await openPalette(canvasElement)
    await userEvent.type(input, 'a')
    await waitFor(() => expect(sectionsOf(dialog).slice(0, 2)).toEqual(['Agents', 'Missions']))
    await userEvent.clear(input)
    await userEvent.type(input, 'new')
    await waitFor(() => expect(selected(dialog)).toBe('new-mission'))
    await userEvent.keyboard('{Enter}')
    await closed()
    await expect(within(canvasElement).getByTestId('ran')).toHaveTextContent('new-mission')
  },
}

/** The empty list shows six rows a section, then Show all; choosing it shows the rest. Scoped, all show at once. */
export const CappedSections: Story = {
  args: { extraAgents: 27 },
  play: async ({ canvasElement }) => {
    const { dialog, input } = await openPalette(canvasElement)
    const agents = () => within(within(dialog).getByRole('group', { name: 'Agents' })).getAllByRole('option')
    await waitFor(() => expect(agents()).toHaveLength(7))
    const more = within(dialog).getByRole('option', { name: 'Show all 30' })
    await userEvent.click(more)
    await waitFor(() => expect(agents()).toHaveLength(30))
    await expect(within(document.body).getByRole('dialog', { name: 'Command palette' })).toBeVisible()
    await userEvent.type(input, '@')
    await waitFor(() => expect(sectionsOf(dialog)).toEqual(['Agents']))
    await expect(agents()).toHaveLength(30)
  },
}

/** A section past two hundred rows renders in chunks near the view; the selected row is always there and in view. */
export const LongSection: Story = {
  args: { extraAgents: 597 },
  play: async ({ canvasElement }) => {
    const { dialog, input } = await openPalette(canvasElement)
    await userEvent.type(input, '@')
    await waitFor(() => expect(dialog.querySelectorAll('[data-palette-chunk="placeholder"]').length).toBeGreaterThan(0))
    await expect(dialog.querySelectorAll('[role="option"]').length).toBeLessThan(300)
    // Up from the first row wraps to the last, which renders and scrolls into view.
    await userEvent.keyboard('{ArrowUp}')
    const last = await within(dialog).findByRole('option', { name: /Worker 597/u })
    await expect(last).toHaveAttribute('aria-selected', 'true')
    await expect(input).toHaveAttribute('aria-activedescendant', last.id)
    const list = within(dialog).getByRole('listbox').getBoundingClientRect(), row = last.getBoundingClientRect()
    await expect(row.top >= list.top - 1 && row.bottom <= list.bottom + 1).toBe(true)
  },
}

/** Nothing matches: the list says so, and Enter does nothing. */
export const NoMatches: Story = {
  play: async ({ canvasElement }) => {
    const { dialog, input } = await openPalette(canvasElement)
    // Unscoped, the space rows that use what was typed always show; scoped to agents, nothing does.
    await userEvent.type(input, '@zzqx')
    await expect(await within(dialog).findByText('No matches.')).toBeVisible()
    await userEvent.keyboard('{Enter}')
    await expect(within(document.body).getByRole('dialog', { name: 'Command palette' })).toBeVisible()
  },
}

/** A prefix scopes to its section; Alt+digit jumps to the TUI's sections; Escape leaves the jump first. */
export const Scoping: Story = {
  play: async ({ canvasElement }) => {
    const { dialog, input } = await openPalette(canvasElement)
    await userEvent.type(input, '@')
    await waitFor(() => expect(sectionsOf(dialog)).toEqual(['Agents']))
    await userEvent.clear(input)
    await userEvent.type(input, '/pal')
    await waitFor(() => expect(sectionsOf(dialog)).toEqual(['Resources']))
    await userEvent.clear(input)
    await userEvent.type(input, '>')
    await waitFor(() => expect(sectionsOf(dialog)).toEqual(['Commands']))
    await userEvent.clear(input)
    await userEvent.keyboard('{Alt>}3{/Alt}')
    await waitFor(() => expect(sectionsOf(dialog)).toEqual(['Missions']))
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(sectionsOf(dialog)).toContain('Agents'))
    await expect(within(document.body).getByRole('dialog', { name: 'Command palette' })).toBeVisible()
  },
}

/** Enter replaces the shown tab; Mod+Enter adds a tab; Alt+Enter splits right; Alt+Shift+Enter below. */
export const Targets: Story = {
  play: async ({ canvasElement }) => {
    const readout = within(canvasElement)
    const mod = /Mac|iPhone|iPad/u.test(navigator.platform) ? 'Meta' : 'Control'
    let { input } = await openPalette(canvasElement)
    await userEvent.type(input, 'release 3')
    await userEvent.keyboard('{Enter}')
    await closed()
    await expect(readout.getByTestId('pane-0')).toHaveTextContent('agent:agent/example/atlas/builder | mission:mission/release-3-2*')
    ;({ input } = await openPalette(canvasElement))
    await userEvent.type(input, 'harbor keeper')
    await userEvent.keyboard(`{${mod}>}{Enter}{/${mod}}`)
    await closed()
    await expect(readout.getByTestId('pane-0')).toHaveTextContent('mission:mission/release-3-2 | agent:agent/example/harbor/keeper*')
    ;({ input } = await openPalette(canvasElement))
    await userEvent.type(input, 'inbox')
    await userEvent.keyboard('{Alt>}{Enter}{/Alt}')
    await closed()
    await expect(readout.getByTestId('pane-1')).toHaveTextContent('list:home*')
    ;({ input } = await openPalette(canvasElement))
    await userEvent.type(input, 'fleet')
    await userEvent.keyboard('{Alt>}{Shift>}{Enter}{/Shift}{/Alt}')
    await closed()
    await expect(readout.getByTestId('pane-2')).toHaveTextContent('list:fleet*')
  },
}

/** New space… asks for a name in the same input; Enter makes it, Escape goes back to the list. */
export const Naming: Story = {
  play: async ({ canvasElement }) => {
    const { dialog, input } = await openPalette(canvasElement)
    await userEvent.type(input, 'new space')
    // As in the TUI, the row using what was typed ranks first; New space… asks for a name instead.
    await waitFor(() => expect(selected(dialog)).toBe('space-new-named'))
    await userEvent.click(within(dialog).getByRole('option', { name: 'New space…' }))
    const name = await within(dialog).findByRole('combobox', { name: 'Name' })
    await expect(name).toHaveValue('')
    await userEvent.type(name, 'Review')
    await expect(within(dialog).getByRole('option')).toHaveTextContent('New space “Review”')
    await userEvent.keyboard('{Enter}')
    await closed()
    await expect(within(canvasElement).getByTestId('space')).toHaveTextContent('Review (2 spaces)')
  },
}

/** Three characters ask st's conversation search; a hit opens that agent's conversation. */
export const SaidInConversations: Story = {
  play: async ({ canvasElement }) => {
    const { dialog, input } = await openPalette(canvasElement)
    await userEvent.type(input, 'harbor keys')
    const row = await within(dialog).findByText('The harbor keys rotate every Monday at nine.')
    await expect(row.closest('[data-section]')).toHaveAttribute('data-section', 'said')
    await expect(within(dialog).getByText(/still indexing, more may come/u)).toBeVisible()
  },
}

/** The phone opens a full-screen sheet from the top bar's search button; it offers only here and a new tab. */
export const Phone: Story = {
  args: { phone: true },
  parameters: { viewport: { defaultViewport: 'mobile1' } },
  play: async ({ canvasElement }) => {
    const { dialog, input } = await openPalette(canvasElement)
    await expect(dialog).toHaveAttribute('data-phone')
    await expect(dialog.querySelector('[data-palette-hints]')).toBeNull()
    await userEvent.click(within(dialog).getByRole('radio', { name: 'New tab' }))
    await userEvent.type(input, 'harbor keeper')
    // Alt+Enter would split; the phone keeps to its pane.
    await userEvent.keyboard('{Alt>}{Enter}{/Alt}')
    await closed()
    await expect(within(canvasElement).queryByTestId('pane-1')).toBeNull()
    await expect(within(canvasElement).getByTestId('pane-0')).toHaveTextContent('agent:agent/example/harbor/keeper*')
  },
}

export const PhoneLight: Story = { args: { phone: true, scheme: 'light' }, parameters: Phone.parameters, play: async ({ canvasElement }) => {
  const { dialog } = await openPalette(canvasElement)
  await expect(within(dialog).getByRole('button', { name: 'Close' })).toBeVisible()
} }

const LivePalette = () => {
  const [state, setState] = React.useState(initial)
  const palette = usePaletteOpen()
  const live = useLivePalette()
  const commands = useLiveHostCommands()
  return <main data-scheme="dark" {...stylex.props(styles.root, liveLegacyTheme, assistantDarkTheme)}>
    <header {...stylex.props(styles.bar)}><button type="button" onClick={palette.open}>Open palette</button></header>
    <CommandPalette {...palette} {...live} commands={commands} state={state} dispatch={command => setState(current => transition(current, command, config))} />
  </main>
}

const LiveFixturesHarness = () => {
  const [registry] = React.useState(() => AtomRegistry.make())
  const [source] = React.useState(() => fixtureSource({ world: fixtureProjections }))
  return <DataSourceProvider source={source} registry={registry}><LivePalette /></DataSourceProvider>
}

/** Bound to the data source: the shared fixture roster's agents, missions and alerts reach the palette. */
export const LiveFixtures: Story = {
  render: () => <LiveFixturesHarness />,
  play: async ({ canvasElement }) => {
    const { dialog, input } = await openPalette(canvasElement)
    await waitFor(() => expect(sectionsOf(dialog)).toEqual(expect.arrayContaining(['Alerts', 'Agents', 'Missions'])))
    // The web has no fleet surface yet, so the kind registry keeps the Fleet rows out.
    await expect(sectionsOf(dialog)).not.toContain('Fleet')
    await userEvent.type(input, '@')
    await waitFor(() => expect(sectionsOf(dialog)).toEqual(['Agents']))
    await expect(within(dialog).getAllByRole('option').length).toBe(fixtureProjections.agents.length)
    // The theme command writes the same preference Settings does.
    await userEvent.clear(input)
    await userEvent.type(input, '>light')
    await waitFor(() => expect(selected(dialog)).toBe('theme-light'))
    await userEvent.keyboard('{Enter}')
    await closed()
    const again = await openPalette(canvasElement)
    await userEvent.type(again.input, '>theme')
    await waitFor(() => expect(again.dialog.querySelector('[data-palette-row="theme-light"]')).toBeNull())
    await expect(within(again.dialog).getByRole('option', { name: /Use dark theme/u })).toBeVisible()
    // Leave the shared preference as the other stories expect it.
    await userEvent.clear(again.input)
    await userEvent.type(again.input, '>use dark')
    await userEvent.keyboard('{Enter}')
    await closed()
  },
}

const styles = stylex.create({
  root: { minHeight: 640, display: 'flex', flexDirection: 'column', gap: s.lg, padding: s.xl, boxSizing: 'border-box', backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  phone: { maxWidth: 440 },
  bar: { display: 'flex', alignItems: 'center', gap: s.md },
  readout: { display: 'flex', flexDirection: 'column', gap: s.xs, fontFamily: t.fontMono, fontSize: t.metaSize },
})
