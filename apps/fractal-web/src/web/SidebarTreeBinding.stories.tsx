import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { decodeUnknownSync, UsageSummary } from '@fractal/client/schema'
import { MilestoneAgentRow, assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import type { Agent } from '../data/source.ts'
import type { SidebarNode } from '../folders/sidebarContract.ts'
import { AgentFolders } from '../shell/AgentFolders.tsx'
import { ResponsiveAgentSidebar } from './ResponsiveAgentSidebar.tsx'
import { sidebarRow } from './sidebarRow.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'

const now = Date.UTC(2026, 9, 8, 12)
const usage = decodeUnknownSync(UsageSummary)({ aggregation: 'cumulative-per-incarnation-else-response-deltas', incarnation_count: 1, input_tokens: 100, output_tokens: 200, cached_tokens: 300, total_tokens: 600, context: { used_percent: 18, observed_at_unix_ms: now } })
const unknown = { _tag: 'Unknown' } as const
const agents: readonly Agent[] = Array.from({ length: 120 }, (_, index) => ({ ref: `agent/sample-${index}`, terminal: '', name: `Sample ${index}`, lifecycle: unknown, host: 'local', connected: true, harness: 'omp', incarnationId: 'inc/sample', activity: index === 0 ? 'working' : 'idle', status: index === 0 ? 'working' : 'idle', usage: { _tag: 'Known', value: usage }, checkout: unknown, workspace: unknown, startedAt: unknown, endedAt: unknown, blockedOn: unknown, ask: unknown, lastActivityAt: { _tag: 'Known', value: now - 60_000 } }))
const agentsByRef = new Map(agents.map(agent => [agent.ref, agent]))
const rows: readonly SidebarNode[] = agents.map(agent => ({ _tag: 'Agent', id: agent.ref, subject: agent.ref, label: agent.name }))
const deepRef = agents.at(-1)!.ref

/** The live roster binding: production frame, folder tree and row, fed by `sidebarRow` projections of fixture agents. */
const TreeBindingSurface = ({ scheme }: { readonly scheme: 'dark' | 'light' }) => {
  const [width, setWidth] = React.useState(256)
  const [selected, setSelected] = React.useState(agents[0]!.ref)
  const [collapsed, setCollapsed] = React.useState(true)
  const tree: readonly SidebarNode[] = [...rows.slice(0, 100), { _tag: 'Folder', id: 'deep', label: 'Deep work', collapsed, children: rows.slice(100) }]
  return <main data-tree-binding-story style={{ width: 1440, height: 700, display: 'flex' }} {...stylex.props(liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
    <ResponsiveAgentSidebar viewport={1440} width={width} maxWidth={440} collapsed={false} drawerOpen={false} onDrawerOpenChange={() => undefined} onToggleCollapsed={() => undefined} onWidthChange={setWidth} onWidthCommit={setWidth} onWidthReset={() => setWidth(256)}>
      <nav aria-label="Agent roster" style={{ flex: '1 1 0', minHeight: 0, display: 'flex', flexDirection: 'column' }}>
        <AgentFolders tree={tree} selectedId={selected} canDrop={() => ({ refused: 'Read-only fixture' })} onMove={() => undefined} onCreateFolder={() => undefined} onRenameFolder={() => undefined} onDeleteFolder={() => undefined} onToggleCollapsed={({ collapsed }) => setCollapsed(collapsed)} onSelect={setSelected} renderAgent={(node, touchDisclosure) => {
          const agent = agentsByRef.get(node.id)!
          return <MilestoneAgentRow row={sidebarRow({ agent, stale: false, now })} reportedStatusOnly active={selected === agent.ref} now={now} touchDisclosure={touchDisclosure} onOpen={() => setSelected(agent.ref)} />
        }} />
      </nav>
    </ResponsiveAgentSidebar>
    <section aria-label="Selected thread"><Aria.Button onPress={() => setSelected(deepRef)}>Open deep thread</Aria.Button><p>{selected}</p></section>
  </main>
}
const meta = {
  title: 'wf/Sidebar/Live tree binding',
  component: TreeBindingSurface,
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Fixed fixture agents projected through the live sidebarRow binding into the production tree, frame and row.' } },
  args: { scheme: 'dark' },
} satisfies Meta<typeof TreeBindingSurface>
export default meta
type Story = StoryObj<typeof meta>
const rowFor = (canvasElement: HTMLElement, ref: string) => canvasElement.querySelector<HTMLElement>(`[data-wf-agent-ref="${ref}"] [data-testid="taste-agent-row"]`)
const play: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  // Reported context stays a popover fact: the row draws no context ring, the hover card names the reported reading.
  const first = rowFor(canvasElement, agents[0]!.ref)!
  await expect(first.querySelector('[data-row-context-ring]')).toBeNull()
  await expect(canvasElement.querySelector('[data-row-context-ring]')).toBeNull()
  await userEvent.hover(first.querySelector<HTMLElement>('[data-row-column="title"]')!)
  const context = await waitFor(() => { const element = document.querySelector('[data-testid="agent-hover-card"] [data-context-observation]'); expect(element).not.toBeNull(); return element! })
  await expect(context).toHaveTextContent(`Reported main-session context: 18%; as of ${new Date(now).toISOString()}`)
  await userEvent.unhover(first.querySelector<HTMLElement>('[data-row-column="title"]')!)
  await waitFor(() => expect(document.querySelector('[data-testid="agent-hover-card"]')).toBeNull())
  // Resizing the production frame 256 → 320 by keyboard widens the row it hosts.
  const separator = canvas.getByRole('separator', { name: 'Agent sidebar width' })
  separator.focus()
  await expect(separator).toHaveFocus()
  await expect(separator).toHaveAttribute('aria-valuenow', '256')
  const narrowWidth = first.getBoundingClientRect().width
  await userEvent.keyboard('{ArrowRight}{ArrowRight}{ArrowRight}{ArrowRight}{ArrowRight}{ArrowRight}{ArrowRight}{ArrowRight}')
  await waitFor(() => expect(separator).toHaveAttribute('aria-valuenow', '320'))
  await waitFor(() => expect(rowFor(canvasElement, agents[0]!.ref)!.getBoundingClientRect().width).toBeGreaterThan(narrowWidth))
  // Selecting a row inside a collapsed folder far below the fold expands it and reveals the row.
  await userEvent.click(canvas.getByRole('button', { name: 'Open deep thread' }))
  const tree = canvas.getByRole('treegrid', { name: 'Agents and folders' })
  const assertRevealed = () => {
    const active = canvasElement.querySelector<HTMLElement>(`[data-wf-agent-ref="${deepRef}"]`)
    if (active === null) throw new Error('Active deep row was not revealed')
    const row = active.getBoundingClientRect(), viewport = tree.getBoundingClientRect()
    if (row.top < viewport.top || row.bottom > viewport.bottom) throw new Error('Active deep row was not revealed')
  }
  await waitFor(assertRevealed)
  // The revealed row paints the active fill that an unselected sibling row does not.
  await expect(getComputedStyle(rowFor(canvasElement, deepRef)!).backgroundColor).not.toBe(getComputedStyle(rowFor(canvasElement, agents.at(-2)!.ref)!).backgroundColor)
  // Negative control: scrolled away, the same assertion fails, so it measures the reveal rather than passing vacuously.
  const scroll = tree.scrollTop
  tree.scrollTop = 0
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  try { await expect(assertRevealed).toThrow('Active deep row was not revealed') } finally { tree.scrollTop = scroll }
  await waitFor(assertRevealed)
}
export const Dark: Story = { play }
export const Light: Story = { args: { scheme: 'light' }, play }
