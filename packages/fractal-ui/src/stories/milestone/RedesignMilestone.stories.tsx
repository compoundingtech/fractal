/** RM1 shell driven by scenario roster, conversation and synchronization slices. */
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { scenarioArgTypes } from '@fractal/scenarios/storybook'
import { loadWorld } from '@fractal/scenarios'
import { scenarioSections } from '../../scenario-transcript.story-harness.ts'
import { ScenarioMilestone, type MilestoneScenarioArgs as Args } from './milestone-scenario.story-harness.tsx'
const selectedConversation = { conversation: 'The shell mounts only the selected conversation; inactive and closed threads are intentionally not rendered. Navigation plays assert selecting another slice-derived thread.' }

const meta = {
  title: 'Fractal UI/Redesign milestone',
  component: ScenarioMilestone,
  parameters: { layout: 'fullscreen', scenario: { _tag: 'slices', slices: ['roster', 'conversation', 'sync'], invariant: selectedConversation } },
  args: { scheme: 'dark', state: 'slice' },
  argTypes: { ...scenarioArgTypes(['roster', 'conversation', 'sync']), scheme: { control: 'inline-radio', options: ['dark', 'light'] }, state: { control: 'select', options: ['slice', 'empty-thread', 'no-match', 'navigation'] } },
  render: (args, context) => <ScenarioMilestone {...args} persistKey={`rm1-${context.id}`} />,
} satisfies Meta<Args>
export default meta
type Story = StoryObj<Args>

const required = <T,>(value: T | null | undefined, label: string): T => {
  if (value === null || value === undefined) throw new Error(`Missing ${label}`)
  return value
}

const world = (id: string) => ({ scenario: { _tag: 'slices', slices: ['roster', 'conversation', 'sync'], world: id } })

const light = (story: Story): Story => ({ ...story, args: { ...story.args, scheme: 'light' } })

export const Slice: Story = { args: { state: 'slice' } }
export const SliceLight = light(Slice)
export const Loading: Story = { parameters: { scenario: { ...world('loading').scenario, invariant: { ...selectedConversation, roster: 'The loading shell intentionally withholds roster rows even when the wire retains a previous snapshot.', sync: 'An initial loading snapshot takes precedence over transport status until the roster and conversation have loaded.' } } } }
export const LoadingLight = light(Loading)
export const EmptyRoster: Story = { parameters: world('empty') }
export const EmptyRosterLight = light(EmptyRoster)
export const EmptyThread: Story = { args: { state: 'empty-thread' }, parameters: world('fleet-mid-refactor') }
export const EmptyThreadLight = light(EmptyThread)
export const NoMatch: Story = { args: { state: 'no-match' }, parameters: { scenario: { _tag: 'slices', slices: ['roster', 'conversation', 'sync'], invariant: { ...selectedConversation, roster: 'The no-match query deliberately filters all roster rows; the empty search result, not every unfiltered agent name, is this shell state.' } } } }
export const NoMatchLight = light(NoMatch)
export const RosterError: Story = { parameters: { scenario: { ...world('failed-sync-subscription-error').scenario, invariant: { ...selectedConversation, roster: 'A failed roster renders its retry notice instead of the retained agent rows; the still-open conversation remains available.' } } } }
export const RosterErrorLight = light(RosterError)
export const ThreadUnavailable: Story = { parameters: { scenario: { ...world('failed-sync-forbidden').scenario, invariant: { ...selectedConversation, roster: 'The forbidden world fails both agents and conversations; the failed roster withholds retained agent rows behind its retry notice.' } } } }
export const ThreadUnavailableLight = light(ThreadUnavailable)
export const Navigation: Story = {
  args: { state: 'navigation' },
  parameters: world('fleet-mid-refactor'),
  play: async ({ canvasElement }) => {
    const title = required(canvasElement.querySelector<HTMLElement>('[data-scenario-navigation-title]')?.dataset.scenarioNavigationTitle, 'scenario navigation title')
    const paletteGlyph = await waitFor(() => required(within(canvasElement).getAllByRole('menuitem').find(item => item.textContent?.includes(title))?.querySelector('svg'), 'palette status glyph'))
    const sidebarRow = [...canvasElement.querySelectorAll<HTMLElement>('[data-milestone-row]')].find(row => row.textContent?.includes(title))
    if (sidebarRow === undefined) return
    await expect(getComputedStyle(paletteGlyph).color).toBe(getComputedStyle(required(sidebarRow.querySelector('[data-row-column="status"] svg'), 'sidebar status glyph')).color)
  },
}
export const NavigationLight = light(Navigation)

const isMac = /Mac|iPhone|iPad/.test(navigator.platform)
const chord = (key: string) => isMac ? `{Meta>}${key}{/Meta}` : `{Control>}${key}{/Control}`
const rows = (root: HTMLElement) => root.querySelectorAll('[data-milestone-row]')


/** Opening a tool's output shows Changes; hiding changes removes it again, including the narrow Changes tab. */
export const HideChanges: Story = {
  tags: ['critical'], args: { state: 'slice' },
  parameters: world('fleet-mid-refactor'),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const page = within(canvasElement.ownerDocument.body)
    const narrow = canvasElement.querySelector('[data-narrow]') !== null
    // Scoped to the pane strip: the DiffPanel carries its own "Changes" view tab.
    const changesTab = () => { const strip = canvas.queryByRole('tablist', { name: 'Open conversations' }); return strip === null ? null : within(strip).queryByRole('tab', { name: 'Changes' }) }
    await expect(await canvas.findByRole('tablist', { name: 'Open conversations' })).toBeVisible()
    const docked = () => canvas.queryByRole('separator', { name: 'Resize changes' })
    // The selected world may carry live or failed work rather than an authored known duration.
    const work = await waitFor(() => required(canvasElement.querySelector<HTMLElement>('[data-testid="work-log"] > button'), 'work log disclosure'))
    if (work.getAttribute('aria-expanded') === 'false') await userEvent.click(work)
    await expect(work).toHaveAttribute('aria-expanded', 'true')
    await userEvent.click(await waitFor(() => required(canvasElement.querySelector<HTMLElement>('[data-tool-status] > button'), 'tool disclosure')))
    await userEvent.click(required((await canvas.findAllByRole('button', { name: 'Show raw input/output' }))[0], 'raw input/output disclosure'))
    await userEvent.click(await canvas.findByRole('button', { name: /^Open .+ tool detail$/ }))
    if (narrow) await waitFor(() => expect(changesTab()).toHaveAttribute('aria-selected', 'true'))
    else await waitFor(() => expect(docked()).not.toBeNull())
    await userEvent.keyboard(chord('k'))
    const palette = await page.findByRole('dialog', { name: 'Command palette' })
    await userEvent.keyboard('hide changes')
    await waitFor(() => expect(within(palette).getAllByRole('menuitem')[0]).toHaveTextContent('Hide changes'))
    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(page.queryByRole('dialog', { name: 'Command palette' })).toBeNull())
    await waitFor(() => expect(narrow ? changesTab() : docked()).toBeNull())
    await expect(canvasElement.querySelector('[aria-label^="Transcript"]')).not.toBeNull()
  },
}

/** Mod+K opens with the first result highlighted, so Enter opens it; "/" filter, tree arrows, Mod+B and the row menu's Open conversation work end to end. */
export const NavigationKeyboard: Story = {
  tags: ['critical'], args: { state: 'slice' },
  parameters: world('fleet-mid-refactor'),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const page = within(canvasElement.ownerDocument.body)
    const active = () => canvasElement.ownerDocument.activeElement
    const title = required(canvasElement.querySelector<HTMLElement>('[data-scenario-navigation-title]')?.dataset.scenarioNavigationTitle, 'scenario navigation title')
    const primaryTitle = required(canvasElement.querySelector<HTMLElement>('[data-scenario-primary-title]')?.dataset.scenarioPrimaryTitle, 'scenario primary title')
    await waitFor(() => expect(canvas.getByRole('main', { name: 'Workspace' })).toBeVisible())
    await userEvent.click(canvas.getByRole('main', { name: 'Workspace' }), { skipHover: true })
    await userEvent.keyboard(chord('k'))
    const palette = await page.findByRole('dialog', { name: 'Command palette' })
    await userEvent.keyboard(title)
    await waitFor(() => expect(within(palette).getAllByRole('menuitem')[0]).toHaveTextContent(title))
    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(page.queryByRole('dialog', { name: 'Command palette' })).toBeNull())
    await waitFor(() => expect(within(canvas.getByRole('tablist', { name: 'Open conversations' })).getByRole('tab', { selected: true })).toHaveTextContent(title))
    await userEvent.keyboard('/')
    const search = await canvas.findByRole('searchbox', { name: 'Filter agents' })
    await waitFor(() => expect(search).toHaveFocus())
    await waitFor(() => expect(rows(canvasElement).length).toBeGreaterThan(3))
    // The roster is one tree: one Tab stop, arrows move between rows, ArrowUp from the first row returns to the filter.
    await userEvent.keyboard('{ArrowDown}')
    await waitFor(() => expect(active()?.getAttribute('role')).toBe('row'))
    const first = active()
    await userEvent.keyboard('{ArrowDown}')
    await waitFor(() => expect(active()).not.toBe(first))
    await userEvent.keyboard('{ArrowUp}{ArrowUp}')
    await waitFor(() => expect(search).toHaveFocus())
    await userEvent.keyboard(chord('b'))
    await waitFor(() => expect(canvas.queryByRole('searchbox', { name: 'Filter agents' })).toBeNull())
    await userEvent.keyboard(chord('b'))
    await waitFor(() => expect(canvas.getByRole('searchbox', { name: 'Filter agents' })).toBeVisible())
    // The row menu opens the same conversation a row press does: enabled, and it lands on the agent's tab.
    await userEvent.click(canvas.getByRole('button', { name: `Actions for ${primaryTitle}` }))
    const rowMenu = await page.findByRole('menu', { name: `Actions for ${primaryTitle}` })
    const openItem = within(rowMenu).getByRole('menuitem', { name: 'Open conversation' })
    await expect(openItem).not.toHaveAttribute('aria-disabled', 'true')
    await userEvent.click(openItem)
    await waitFor(() => expect(page.queryByRole('menu')).toBeNull())
    await waitFor(() => expect(within(canvas.getByRole('tablist', { name: 'Open conversations' })).getByRole('tab', { selected: true })).toHaveTextContent(primaryTitle))
  },
}


/** Preserve the world's actual transport issue and retry attempt, not the authored stale fallback. */
export const ReconnectingSync: Story = {
  parameters: world('failed-sync-open-fail'),
  play: async ({ canvasElement }) => {
    const expected = loadWorld('failed-sync-open-fail').slices.sync.state.expected.find(entry => entry.surface.startsWith('conversation:'))?.status
    if (expected?._tag !== 'Stale' || expected.reason._tag !== 'Reconnecting') throw new Error('Open-fail world must retain a reconnecting conversation status')
    const banner = await waitFor(() => {
      const element = canvasElement.querySelector<HTMLElement>('[data-connection-state="reconnecting"]')
      expect(element).not.toBeNull()
      if (element === null) throw new Error('Missing reconnecting connection banner')
      return element
    })
    await expect(banner).toHaveTextContent(expected.reason.issue)
    await expect(banner).toHaveTextContent(`attempt ${expected.reason.attempt}`)
    await expect(canvasElement).not.toHaveTextContent('Gateway connection interrupted')
  },
}

/** Mounting committed messages must not relabel an incomplete server history as Complete. */
export const HugeHistory: Story = {
  parameters: world('long-debug-rabbit-hole'),
  play: async ({ canvasElement }) => {
    const sections = scenarioSections(loadWorld('long-debug-rabbit-hole').slices.conversation, 'whole-thread').sections
    await expect(sections[0]?.history?._tag).toBe('HasOlder')
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Show all messages' }))
    // This world's 105 committed entries retain has_more=true without the huge variant's 1,600-row reveal.
    await waitFor(() => expect(canvas.getByTestId('history-boundary-note')).toHaveTextContent('Earlier messages not loaded'), { timeout: 5_000 })
  },
}
