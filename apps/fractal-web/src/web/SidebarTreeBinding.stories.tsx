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
import { ThemePortal } from '../../../../packages/fractal-ui/src/assistant-ui/taste/ThemePortal.tsx'

const now = Date.UTC(2026, 9, 8, 12)
const usage = decodeUnknownSync(UsageSummary)({ aggregation: 'cumulative-per-incarnation-else-response-deltas', incarnation_count: 1, input_tokens: 100, output_tokens: 200, cached_tokens: 300, total_tokens: 600, context: { used_percent: 18, observed_at_unix_ms: now } })
const unknown = { _tag: 'Unknown' } as const
const makeAgent = (ref: string, name: string, working: boolean): Agent => ({ ref, terminal: '', name, lifecycle: unknown, host: 'local', connected: true, harness: 'omp', incarnationId: 'inc/sample', activity: working ? 'working' : 'idle', status: working ? 'working' : 'idle', usage: { _tag: 'Known', value: usage }, checkout: unknown, workspace: unknown, startedAt: unknown, endedAt: unknown, blockedOn: unknown, ask: unknown, lastActivityAt: { _tag: 'Known', value: now - 60_000 } })
const agents: readonly Agent[] = Array.from({ length: 120 }, (_, index) => makeAgent(`agent/sample-${index}`, `Sample ${index}`, index === 0))
// Long names wider than any dock width, in many project folders: twelve folders of six seats, the roster scrolls.
const longNames = ['Row projection reconciler for long-running workspace sessions', 'GatewaySchemaMigrationReviewerForTheFleetRollout', 'Palette review of every responsive sidebar breakpoint state', 'TerminalReplayCompatibilityVerifier', 'Workbench shell keyboard navigation audit', 'Mission queue fairness and starvation investigation']
const longFolders = Array.from({ length: 12 }, (_, folder) => ({
  label: `Fleet infrastructure and long-running gateway migrations ${folder + 1}`,
  agents: longNames.map((name, index) => makeAgent(`agent/long-${folder}-${index}`, `${name} ${folder + 1}`, folder === 0 && index === 0)),
}))
const longAgent = longFolders[0]!.agents[0]!
const agentsByRef = new Map([...agents, ...longFolders.flatMap(folder => folder.agents)].map(agent => [agent.ref, agent]))
const rows: readonly SidebarNode[] = agents.map(agent => ({ _tag: 'Agent', id: agent.ref, subject: agent.ref, label: agent.name }))
const deepRef = agents.at(-1)!.ref

/** The live roster binding: production frame, folder tree and row, fed by `sidebarRow` projections of fixture agents. */
const TreeBindingSurface = ({ scheme, variant = 'default' }: { readonly scheme: 'dark' | 'light'; readonly variant?: 'default' | 'long-names-many-folders' }) => {
  const long = variant === 'long-names-many-folders'
  const [width, setWidth] = React.useState(256)
  const [selected, setSelected] = React.useState(long ? longAgent.ref : agents[0]!.ref)
  const [collapsed, setCollapsed] = React.useState(true)
  // Every long folder starts open except the last two, so the roster both scrolls and shows collapsed folders.
  const [collapsedFolders, setCollapsedFolders] = React.useState<ReadonlySet<string>>(() => new Set(['long-10', 'long-11']))
  const tree: readonly SidebarNode[] = long
    ? longFolders.map((folder, index) => ({ _tag: 'Folder', id: `long-${index}`, label: folder.label, collapsed: collapsedFolders.has(`long-${index}`), children: folder.agents.map(agent => ({ _tag: 'Agent', id: agent.ref, subject: agent.ref, label: agent.name })) }))
    : [...rows.slice(0, 100), { _tag: 'Folder', id: 'deep', label: 'Deep work', collapsed, children: rows.slice(100) }]
  return <main data-tree-binding-story style={{ width: 1440, height: 700, display: 'flex' }} {...stylex.props(liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
    <ThemePortal>
    <ResponsiveAgentSidebar viewport={1440} width={width} maxWidth={440} collapsed={false} drawerOpen={false} onDrawerOpenChange={() => undefined} onToggleCollapsed={() => undefined} onWidthChange={setWidth} onWidthCommit={setWidth} onWidthReset={() => setWidth(256)}>
      <nav aria-label="Agent roster" style={{ flex: '1 1 0', minHeight: 0, display: 'flex', flexDirection: 'column' }}>
        <AgentFolders tree={tree} selectedId={selected} canDrop={() => ({ refused: 'Read-only fixture' })} onMove={() => undefined} onCreateFolder={() => undefined} onRenameFolder={() => undefined} onDeleteFolder={() => undefined} onToggleCollapsed={change => {
          if (!long) { setCollapsed(change.collapsed); return }
          setCollapsedFolders(previous => { const next = new Set(previous); if (change.collapsed) next.add(change.id); else next.delete(change.id); return next })
        }} onSelect={setSelected} renderAgent={(node, touchDisclosure) => {
          const agent = agentsByRef.get(node.id)!
          return <MilestoneAgentRow row={sidebarRow({ agent, stale: false, now })} reportedStatusOnly active={selected === agent.ref} now={now} touchDisclosure={touchDisclosure} onOpen={() => setSelected(agent.ref)} />
        }} />
      </nav>
    </ResponsiveAgentSidebar>
    <section aria-label="Selected thread"><Aria.Button onPress={() => setSelected(deepRef)}>Open deep thread</Aria.Button><p>{selected}</p></section>
    </ThemePortal>
  </main>
}
const meta = {
  title: 'wf/Sidebar/Live tree binding',
  component: TreeBindingSurface,
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Fixed fixture agents projected through the live sidebarRow binding into the production tree, frame and row.' } },
  args: { scheme: 'dark', variant: 'default' },
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
  await expect(context.closest('[data-milestone-row]')).toBeNull()
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
    const active = rowFor(canvasElement, deepRef)
    if (active === null) throw new Error('Active deep row was not revealed')
    const row = active.getBoundingClientRect(), viewport = tree.getBoundingClientRect()
    if (row.top < viewport.top || row.bottom > viewport.bottom) throw new Error('Active deep row was not revealed')
  }
  await waitFor(assertRevealed)
  // The revealed row paints the active fill that an unselected sibling row does not.
  await expect(getComputedStyle(rowFor(canvasElement, deepRef)!).backgroundColor).not.toBe(getComputedStyle(rowFor(canvasElement, agents.at(-2)!.ref)!).backgroundColor)
  // Negative control: scrolled away, the same assertion fails, so it measures the reveal rather than passing vacuously.
  const scroll = tree.scrollTop
  tree.dispatchEvent(new WheelEvent('wheel', { bubbles: true }))
  tree.scrollTop = 0
  try { await waitFor(() => expect(assertRevealed).toThrow('Active deep row was not revealed')) } finally { tree.scrollTop = scroll }
  await waitFor(assertRevealed)
}
export const Dark: Story = { tags: ['critical'], play }
export const Light: Story = { tags: ['critical'], args: { scheme: 'light' }, play }

/**
 * The production binding with long names in many folders: the roster scrolls vertically and never sideways, every
 * long title ellipsizes inside its row, and the last folder is reachable by scrolling.
 */
const longPlay: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const tree = canvas.getByRole('treegrid', { name: 'Agents and folders' })
  const first = await waitFor(() => { const row = rowFor(canvasElement, longAgent.ref); expect(row).not.toBeNull(); return row! })
  const title = first.querySelector<HTMLElement>('[data-row-column="title"]')!
  const box = first.getBoundingClientRect()
  await expect(title.getBoundingClientRect().right).toBeLessThanOrEqual(box.right + 0.5)
  await expect([title, ...title.querySelectorAll<HTMLElement>('*')].some(node => node.scrollWidth > node.clientWidth)).toBe(true)
  await expect(tree.scrollWidth).toBeLessThanOrEqual(tree.clientWidth)
  await expect(tree.scrollHeight).toBeGreaterThan(tree.clientHeight)
  // The tree is virtualized and measures rows as they mount, so its scroll height grows while it settles: keep
  // scrolling to the end until the last folder's row is rendered and painted inside the tree's viewport.
  const lastKey = `long-${longFolders.length - 1}`
  await waitFor(() => {
    tree.scrollTop = tree.scrollHeight
    const last = tree.querySelector<HTMLElement>(`[data-sidebar-tree-key="${lastKey}"]`)
    expect(last).not.toBeNull()
    const row = last!.getBoundingClientRect(), viewport = tree.getBoundingClientRect()
    expect(row.top).toBeGreaterThanOrEqual(viewport.top - 0.5)
    expect(row.bottom).toBeLessThanOrEqual(viewport.bottom + 0.5)
    expect(last!.textContent).toContain(longFolders.at(-1)!.label)
  }, { timeout: 5000 })
  tree.scrollTop = 0
}
export const LongNamesManyFolders: Story = {
  tags: ['critical'],
  args: { variant: 'long-names-many-folders' },
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Fixed long-named fixture agents in twelve folders, projected through the live sidebarRow binding, pin truncation and vertical-only scrolling in the production tree.' } },
  play: longPlay,
}
export const LongNamesManyFoldersLight: Story = { ...LongNamesManyFolders, args: { ...LongNamesManyFolders.args, scheme: 'light' } }
