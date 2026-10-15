import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import axe from 'axe-core'
import { assistantDarkTheme, compositionLightTheme } from '@smalltalk/fractal-ui/assistant-ui/shell'
import { surfaceVars as surface, textVars as ink, typeVars as type, spaceVars as space } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { Agent } from '../data/source.ts'
import { searchAgents } from '../web/sidebarSearch.fixtures.ts'
import { CommandPalette } from './CommandPalette.tsx'
import { commandSections, switchSections, type PaletteCommand, type PaletteMode, type PaletteRow } from './model.ts'

// Live roster names, folders and hosts from 2026-10-12, repeated with numbered copies to reach a realistic size.
const names = [
  ['Fleet coordinator', 'Fleet / manager', 'dev5'], ['Fleet storage steward', 'Fleet / manager', 'dev3'], ['Janitor', 'Fleet / Runtime & hosts', 'dev3'],
  ['Global complexity orchestrator', 'Agent system / Agent control', 'dev3'], ['Agent attention', 'Agent system / Agent control', 'dev3'],
  ['Buck2 migration lead', 'Engineering loop / buck2', 'dev3'], ['Graph-as-code steward', 'Compounding / Smalltalk', 'dev5'],
  ['Notion IaC Alchemy prototype', 'Knowledge / Context', 'dev3'], ['Berlin home owner', 'Life / Home', 'dev3'], ['Eggisried home owner', 'Life / Home', 'dev3'],
  ['UI design', 'Design', 'dev5'], ['Investigating cross-host agent sessions with exceptionally long mission and workspace names', 'Research / Long-running missions', 'dev4'],
] as const
const agents: readonly Agent[] = Array.from({ length: 6 }, (_, copy) => names.map(([name, , host], index) => ({
  ...searchAgents[0]!, ref: `agent/${index}-${copy}`, name: copy === 0 ? name : `${name} ${copy + 1}`, host, connected: host !== 'dev4', status: copy % 2 === 0 ? 'working' : 'idle', mission: index === 0 ? 'mission/dotfiles/fleet' : undefined,
}))).flat()
const folders = new Map(agents.map((agent, index) => [agent.ref, names[index % names.length]![1]]))
const missions = [{ id: 'mission/dotfiles/fleet', title: 'Fleet health sweep', state: 'active' }, { id: 'mission/dotfiles/ui-design/palette-build', title: 'Palette build', state: { raw: 'ready' } }]
const commands: readonly PaletteCommand[] = [
  { id: 'attention.open', title: 'Open pending questions', detail: '3 agents ask a question', category: 'Navigate', keywords: ['needs you', 'inbox'] },
  { id: 'terminal.toggle', title: 'Open terminal', detail: 'Fleet coordinator', category: 'View' },
  { id: 'details.toggle', title: 'Show agent details', detail: 'Fleet coordinator', category: 'View' },
  { id: 'sidebar.toggle', title: 'Toggle sidebar', detail: 'Agent roster', category: 'View', shortcut: 'Ctrl B' },
  { id: 'theme.toggle', title: 'Use light theme', detail: 'This device', category: 'View' },
  { id: 'link.copy', title: 'Copy link to this agent', detail: 'Fleet coordinator', category: 'Share', disabledReason: 'Select an agent first.' },
]

interface PaletteStoryArgs {
  readonly scheme: 'dark' | 'light'
  readonly initialQuery: string
  readonly initialMode: PaletteMode
  readonly inventoryNote?: string
}

/** The production palette over a fixed roster; Enter announces the chosen row instead of navigating. */
function PaletteStory({ scheme, initialQuery, initialMode, inventoryNote }: PaletteStoryArgs) {
  const [open, setOpen] = React.useState(true)
  const [mode, setMode] = React.useState(initialMode)
  const [query, setQuery] = React.useState(initialQuery)
  const [recents, setRecents] = React.useState<readonly string[]>(['agent/4-0', 'agent/1-0'])
  const [announcement, setAnnouncement] = React.useState('')
  const [portal, setPortal] = React.useState<HTMLDivElement | null>(null)
  const opener = React.useRef<HTMLButtonElement>(null)
  const sections = mode === 'Commands' ? commandSections({ query, commands })
    : switchSections({ query, agents, folders, needsYou: new Set(['agent/2-0', 'agent/6-0', 'agent/3-1']), recents, current: 'agent/4-0', missions, statusLabel: agent => agent.connected ? agent.status : 'Offline · idle' })
  const choose = (row: PaletteRow) => {
    setOpen(false)
    if (row.target._tag === 'Agent') setRecents(value => [row.id, ...value.filter(ref => ref !== row.id)])
    setAnnouncement(`Opened ${row.title}`)
  }
  React.useEffect(() => { if (!open) opener.current?.focus() }, [open])
  return <div ref={setPortal} {...stylex.props(styles.root, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
    <Aria.Button ref={opener} onPress={() => { setMode('Switch'); setQuery(''); setOpen(true) }} {...stylex.props(styles.opener)}>Switch agent</Aria.Button>
    <p role="status">{announcement}</p>
    {portal === null ? null : <CommandPalette isOpen={open} onOpenChange={setOpen} mode={mode} onModeChange={next => { setMode(next); setQuery('') }} query={query} onQueryChange={setQuery} sections={sections} onSelect={choose} shortcut="Ctrl K" portalContainer={portal} {...(inventoryNote === undefined ? {} : { inventoryNote })} />}
  </div>
}

const styles = stylex.create({
  root: { minHeight: '100dvh', padding: space.xl, backgroundColor: surface.canvas, color: ink.fg, fontFamily: type.fontSans, fontSize: type.uiSize },
  opener: { padding: space.md, backgroundColor: surface.raised, color: ink.fg, borderWidth: 1, borderStyle: 'solid', borderColor: surface.rowActive, borderRadius: 6 },
})

const meta = {
  title: 'wf/Command palette',
  component: PaletteStory,
  args: { scheme: 'dark', initialQuery: '', initialMode: 'Switch' },
  argTypes: { scheme: { control: 'radio', options: ['dark', 'light'] }, initialMode: { control: 'radio', options: ['Switch', 'Commands'] } },
  parameters: { layout: 'fullscreen', docs: { description: { component: 'P2·G1: top-anchored dialog with Needs you pinned, recent agents, all agents and missions; Commands is the adjacent mode.' } } },
} satisfies Meta<typeof PaletteStory>
export default meta
type Story = StoryObj<typeof meta>

export const Switch: Story = {}
export const Light: Story = { args: { scheme: 'light' } }
export const Query: Story = { args: { initialQuery: 'fl st' } }
export const NoResults: Story = { args: { initialQuery: 'unfindable xyz', inventoryNote: 'Showing a partial roster.' } }
export const LongName: Story = { args: { initialQuery: 'exceptionally' } }
export const Commands: Story = { args: { initialMode: 'Commands' } }

const searchbox = (canvasElement: HTMLElement) => within(canvasElement).findByRole('searchbox', { name: 'Search agents, missions and commands' })

export const EscapeRestoresInvoker: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const input = await searchbox(canvasElement)
    await userEvent.type(input, 'fleet')
    await userEvent.keyboard('{ArrowDown}')
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(canvas.queryByRole('dialog')).not.toBeInTheDocument())
    await waitFor(() => expect(canvas.getByRole('button', { name: 'Switch agent' })).toHaveFocus())
  },
}

/**
 * The announced row (`aria-activedescendant`) must be the row React Aria focuses and Enter activates. Without
 * `disableAutoFocusFirst`, Autocomplete delays the attribute for an unchanged first row and this test fails.
 */
export const ActiveDescendantSync: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const input = await searchbox(canvasElement)
    // React Aria moves virtual focus in a passive effect one commit after data-focused; 150 ms still rejects
    // Autocomplete's 500 ms delayed activedescendant.
    const assertSync = () => waitFor(() => expect(input.getAttribute('aria-activedescendant') ?? null).toBe(canvasElement.querySelector('[role=menuitem][data-focused]')?.id ?? null), { timeout: 150, interval: 10 })
    await userEvent.type(input, 'steward')
    await assertSync()
    for (let index = 0; index < 4; index++) {
      await userEvent.keyboard('{ArrowDown}')
      await assertSync()
      if (index === 0) await expect(input.getAttribute('aria-activedescendant')).toBe(canvasElement.querySelector('[role=menuitem]')?.id)
    }
    const announced = document.getElementById(input.getAttribute('aria-activedescendant')!)
    const title = announced?.querySelector('[slot=label]')?.textContent
    await expect(title).toBeTruthy()
    await userEvent.keyboard('{Enter}')
    await expect(canvas.getByRole('status')).toHaveTextContent(`Opened ${title}`)
  },
}

/** Switching collections must never leave the searchbox naming a removed virtual-focus row. */
export const ModeSwitchResetsVirtualFocus: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await searchbox(canvasElement), 'fleet')
    // Reach beyond the shorter Commands collection so a recycled row id cannot mask stale focus.
    for (let index = 0; index < 10; index++) {
      await userEvent.keyboard('{ArrowDown}')
      await waitFor(async () => {
        const input = await searchbox(canvasElement)
        expect(input.getAttribute('aria-activedescendant')).toBe(canvasElement.querySelector('[role=menuitem][data-focused]')?.id)
      })
    }
    await waitFor(async () => expect((await searchbox(canvasElement)).getAttribute('aria-activedescendant')).toBeTruthy())
    const settleCollection = async () => {
      const { promise, resolve } = Promise.withResolvers<void>()
      setTimeout(resolve, 300)
      await promise
    }
    const assertValid = () => waitFor(async () => {
      // The mode-keyed Autocomplete remounts the searchbox as well as its collection.
      const input = await searchbox(canvasElement)
      const active = input.getAttribute('aria-activedescendant')
      await expect(active === null || document.getElementById(active)?.getAttribute('role') === 'menuitem').toBe(true)
    }, { timeout: 150, interval: 10 })
    await userEvent.click(canvas.getByRole('button', { name: 'Commands' }))
    // Check settled virtual focus, not the transient null attribute during RAC's collection update.
    await settleCollection()
    await assertValid()
    await userEvent.click(canvas.getByRole('button', { name: 'Switch' }))
    await settleCollection()
    await assertValid()
  },
}

/** Shortcut chips must retain small-text contrast on the focused Commands surface in both themes. */
export const FocusedCommandChipDark: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await searchbox(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: 'Commands' }))
    await userEvent.type(await searchbox(canvasElement), 'sidebar')
    await userEvent.keyboard('{ArrowDown}')
    await waitFor(() => {
      const row = canvasElement.querySelector('[role=menuitem][data-focused]')
      expect(row).toHaveTextContent('Toggle sidebar')
      expect(row?.querySelector('kbd')).toHaveTextContent('Ctrl B')
    })
    const results = await axe.run(canvas.getByRole('dialog'))
    await expect(results.violations.filter(({ impact }) => impact === 'serious' || impact === 'critical')).toEqual([])
  },
}

export const FocusedCommandChipLight: Story = {
  args: { scheme: 'light' },
  play: FocusedCommandChipDark.play,
}
