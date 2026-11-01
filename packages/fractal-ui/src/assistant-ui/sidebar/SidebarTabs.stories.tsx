/** Sidebar tabs over the fleet-mid-refactor world: Agents, Alerts, Missions and Fleet in one row over a fixed-width sidebar, at 440 and 1440 in both themes, beside the records each marker counts. Leading strongest marker, counts dropping before any label truncates, keyboard selection and a bounded panel. */
import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import * as stylex from '@stylexjs/stylex'
import { useScenarioSlice, type WireSlice } from '@fractal/scenarios/react'
import { scenarioArgTypes } from '@fractal/scenarios/storybook'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { SidebarTabs, sidebarTabName, type SidebarTab, type SidebarTabGlyph } from './SidebarTabs'
import { SidebarAgentRow } from './SidebarAgentRow'
import { statuses, type AgentStatus, type SidebarAgentRow as Row } from './model'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { lightTheme, type Scheme } from '../composition-theme'
import { accentVars as accent, borderVars as border, geometryVars as g, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

type WireAgent = WireSlice<'roster'>['state']['agents'][number]
type WireMachine = WireSlice<'roster'>['state']['machines'][number]
type WireCard = WireSlice<'attention'>['state']['attention'][number]
type Attention = 'question' | 'needs-you'

const agentStatus = (state: WireAgent['state']): AgentStatus => {
  switch (state) {
    case 'running': return 'working'
    case 'waiting': return 'waiting'
    case 'desired': case 'starting': return 'pending'
    case 'down': return 'offline'
    case 'stopped': case 'failed': return 'ended'
    case 'suspended': return 'suspended'
    case 'retired': return 'retired'
    default: return 'unobserved'
  }
}
/** An open harness prompt asks a question; every other open card needs the person. */
const cardAttention = (card: WireCard): Attention => card.attention_kind === 'harness-prompt' ? 'question' : 'needs-you'
/** The strongest attention among cards: a question beats needs-you. */
const strongest = (cards: readonly WireCard[]): Attention | undefined => cards.length === 0 ? undefined : cards.some(card => cardAttention(card) === 'question') ? 'question' : 'needs-you'
const machineGlyph = (machine: WireMachine): SidebarTabGlyph => machine.state === 'unreachable' ? (machine.work.length > 0 ? '!' : '×') : machine.state === 'dial-out' ? '○' : machine.state === 'indeterminate' ? '~' : '●'

interface Item { readonly id: string; readonly glyph: string; readonly title: string; readonly attention?: Attention; readonly metric?: string; readonly facts: string }
interface Group { readonly id: string; readonly title: string; readonly attention?: Attention; readonly items: readonly Item[] }
interface Sidebar { readonly tabs: readonly SidebarTab[]; readonly agents: readonly Row[]; readonly alerts: readonly Item[]; readonly missions: readonly Item[]; readonly fleet: readonly Item[]; readonly agentSources: readonly Group[]; readonly missionSteps: readonly Group[] }

/** The brief's step cells: done ■, working ▣, needs you ◆, blocked or failed !, otherwise □. */
const stepGlyphs: Partial<Record<string, string>> = { completed: '■', claimed: '▣', verifying: '▣', 'waiting-person': '◆', blocked: '!', failed: '!' }

/** One projection for the tab markers and the panels, so a marker always counts what its panel shows. */
function useSidebar(): Sidebar {
  const roster = useScenarioSlice('roster')
  const details = useScenarioSlice('details')
  const attention = useScenarioSlice('attention')
  return React.useMemo((): Sidebar => {
    const cards = attention.loading ? [] : attention.state.attention.filter(card => card.state === 'open')
    const machines = roster.loading ? [] : roster.state.machines
    const messages = attention.loading ? [] : attention.state.messages
    const work = details.loading ? [] : details.state.work
    const hosts = new Map(machines.map(machine => [machine.host_id, machine.name]))
    const agents = (roster.loading ? [] : roster.state.agents).map((agent): Row & { readonly attention?: Attention } => {
      const status = agentStatus(agent.state)
      const marker = strongest(cards.filter(card => card.requester_id === agent.id))
      return {
        ref: agent.id, id: agent.id, title: agent.name, status, statusLabel: statuses[status].label, host: agent.host_id == null ? 'Unknown host' : hosts.get(agent.host_id) ?? agent.host_id,
        freshness: 'live', usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Unknown' }, children: [], needsMe: marker !== undefined, ...(marker === undefined ? {} : { attention: marker }),
      }
    })
    const alerts = cards.map((card): Item => ({ id: card.id, glyph: cardAttention(card) === 'question' ? '?' : '◆', title: card.title, attention: cardAttention(card), facts: card.detail }))
    const missions = (details.loading ? [] : details.state.missions).map((mission): Item => {
      const marker = strongest(cards.filter(card => card.mission_id === mission.id))
      return { id: mission.id, glyph: mission.state === 'running' ? '●' : '○', title: mission.title, ...(marker === undefined ? {} : { attention: marker }), facts: mission.state }
    })
    // Each agent's ◆ counts the cards it requested; its mail is listed but raises no marker. Each mission's marker sits over its steps.
    const agentSources = agents.flatMap((agent): Group[] => {
      const items = [...cards.flatMap((card, index) => card.requester_id === agent.id ? [alerts[index]!] : []), ...messages.filter(message => message.from === agent.id).map((message): Item => ({ id: message.id, glyph: '·', title: message.title, facts: `Mail, no marker · ${message.content}` }))]
      return items.length === 0 ? [] : [{ id: agent.id, title: agent.title, ...(agent.attention === undefined ? {} : { attention: agent.attention }), items }]
    })
    const missionSteps = missions.map(({ id, title, attention: marker }): Group => ({
      id, title, ...(marker === undefined ? {} : { attention: marker }),
      items: work.filter(item => item.mission_id === id).map((item): Item => ({ id: item.id, glyph: stepGlyphs[item.state] ?? '□', title: item.title, facts: item.blocked_reason == null ? item.state : `${item.state} · ${item.blocked_reason}` })),
    }))
    const fleet = machines.map((machine): Item => ({ id: machine.id, glyph: machineGlyph(machine), title: machine.name, metric: `${machine.occupancy.running_runtimes} running`, facts: `${machine.state} · ${machine.projects.length} projects · ${machine.work.length} work` }))
    const counted = (id: string, label: string, items: readonly { readonly attention?: Attention }[]): SidebarTab => ({
      id, label, questionCount: items.filter(item => item.attention === 'question').length, needsYouCount: items.filter(item => item.attention === 'needs-you').length,
    })
    const faulted = fleet.filter(item => item.glyph === '!').length
    return {
      tabs: [counted('agents', 'Agents', agents), counted('alerts', 'Alerts', alerts), counted('missions', 'Missions', missions), { id: 'fleet', label: 'Fleet', ...(faulted === 0 ? {} : { status: { glyph: '!', label: `${faulted} unreachable with work` } }) }],
      agents, alerts, missions, fleet, agentSources, missionSteps,
    }
  }, [roster, details, attention])
}

function Suffix({ attention }: { readonly attention?: Attention }) {
  return attention === undefined ? null : <span {...stylex.props(attention === 'question' ? styles.question : styles.needsYou)}>{attention === 'question' ? '?' : '◆'}</span>
}

function ItemList({ label, items }: { readonly label: string; readonly items: readonly Item[] }) {
  return <ul aria-label={label} {...stylex.props(styles.list)}>{items.map(item => <li key={item.id} data-sidebar-item={item.attention ?? 'none'} {...stylex.props(styles.item)}>
    <span {...stylex.props(styles.line)}>
      <span aria-hidden {...stylex.props(styles.glyph, item.glyph === '!' && styles.fault)}>{item.glyph}</span>
      <span data-sidebar-item-title {...stylex.props(styles.title)}>{item.title}</span>
      <Suffix attention={item.attention} />
      {item.metric === undefined ? null : <span {...stylex.props(styles.metric)}>{item.metric}</span>}
    </span>
    <span {...stylex.props(styles.facts)}>{item.facts}</span>
  </li>)}</ul>
}

/** Beside the sidebar, outside its panels: the cards and mail behind each agent's marker, and each mission's steps. */
function MarkerSources({ sidebar }: { readonly sidebar: Sidebar }) {
  const groups = [...sidebar.agentSources, ...sidebar.missionSteps]
  return groups.length === 0 ? null : <section aria-label="Marker sources" {...stylex.props(styles.specimen, styles.sources)}>
    {groups.map(group => <div key={group.id}>
      <p {...stylex.props(styles.line, styles.caption)}><span {...stylex.props(styles.title)}>{group.title}</span><Suffix attention={group.attention} /></p>
      <ItemList label={group.title} items={group.items} />
    </div>)}
  </section>
}

/** The host owns selection and supplies only the selected tab's content, as the app does. */
function SidebarTabsHost({ scheme, sidebar }: { readonly scheme: Scheme; readonly sidebar: Sidebar }) {
  const [selectedId, setSelectedId] = React.useState('agents')
  return <section aria-label={`Sidebar tabs ${scheme}`} data-scheme={scheme} {...stylex.props(styles.specimen, scheme === 'light' && lightTheme)}>
    <output data-testid="sidebar-tab-selection" {...stylex.props(styles.output)}>{selectedId}</output>
    <aside aria-label={`Agents sidebar ${scheme}`} data-sidebar-host {...stylex.props(styles.sidebar)}>
      <SidebarTabs tabs={sidebar.tabs} selectedId={selectedId} onSelectionChange={setSelectedId}>
        <div data-sidebar-scroller {...stylex.props(styles.scroller)}>
          {selectedId === 'agents' ? <div role="list" aria-label="Agents">{sidebar.agents.map(agent => <div role="listitem" key={agent.ref}><SidebarAgentRow item={agent} now={0} /></div>)}</div>
            : selectedId === 'alerts' ? <ItemList label="Alerts" items={sidebar.alerts} />
            : selectedId === 'missions' ? <ItemList label="Missions" items={sidebar.missions} />
            : <ItemList label="Fleet" items={sidebar.fleet} />}
        </div>
      </SidebarTabs>
    </aside>
  </section>
}

type Args = { readonly scheme: Scheme; readonly both?: boolean; readonly roster?: string; readonly details?: string; readonly attention?: string }
/** Below the app's drawer breakpoint the sidebar is the phone drawer: touch-sized tabs at the same sidebar width. */
function SidebarTabsStory({ scheme, both = false }: Args) {
  const sidebar = useSidebar()
  return <NarrowLayoutContext.Provider value={window.innerWidth < 900}>
    <main {...stylex.props(styles.canvas, !both && scheme === 'light' && lightTheme)}>
      {(both ? ['dark', 'light'] as const : [scheme]).map(value => <SidebarTabsHost key={value} scheme={value} sidebar={sidebar} />)}
      <MarkerSources sidebar={sidebar} />
    </main>
  </NarrowLayoutContext.Provider>
}

// fleet-mid-refactor with attention.sidebar-tabs: builder's session asks one question; reviewer (holding the mission), migrator and docs each need you.
const names = ['Agents, 3 need you', 'Alerts, 1 open question', 'Missions, 1 needs you', 'Fleet']
const withCounts = ['◆3 Agents', '? Alerts', '◆ Missions', 'Fleet']
const withoutCounts = ['◆ Agents', '? Alerts', '◆ Missions', 'Fleet']
const leading = ['needs-you', 'question', 'needs-you', 'label']

/** What the marker paints: its ::before `content`, with any unresolved attr() read from the marker. */
const markerText = (marker: Element) => {
  const content = getComputedStyle(marker, '::before').content
  if (content === 'none' || content === 'normal') return ''
  return [...content.matchAll(/attr\(\s*([\w-]+)\s*\)|"((?:[^"\\]|\\.)*)"/g)].map(([, attr, text]) => attr === undefined ? text!.replace(/\\(.)/g, '$1') : marker.getAttribute(attr) ?? '').join('')
}
/** Each tab as painted: the CSS-drawn marker, then the tab's DOM text (one space and the label). */
const visibleText = (tablist: HTMLElement) => within(tablist).getAllByRole('tab').map(tab => {
  const marker = tab.querySelector('[data-sidebar-tab-marker]')
  return (marker === null ? '' : markerText(marker)) + tab.textContent
})

/** Every tab label paints whole: nothing in the row overflows, and the tabs end at the bar's content edge. */
async function assertNoClip(tablist: HTMLElement) {
  await expect(tablist.scrollWidth).toBeLessThanOrEqual(tablist.clientWidth)
  const widths = within(tablist).getAllByRole('tab').map(tab => tab.getBoundingClientRect().width)
  await expect(Math.max(...widths) - Math.min(...widths)).toBeLessThanOrEqual(1)
  if (window.innerWidth < 900) for (const width of widths) await expect(width).toBeGreaterThanOrEqual(44)
  for (const label of tablist.querySelectorAll<HTMLElement>('[data-sidebar-tab-label]')) {
    const tab = label.closest<HTMLElement>('[role="tab"]')!.getBoundingClientRect(), box = label.getBoundingClientRect()
    await expect(box.left).toBeGreaterThanOrEqual(tab.left)
    await expect(box.right).toBeLessThanOrEqual(tab.right + 0.5)
  }
  const bar = tablist.getBoundingClientRect(), last = within(tablist).getAllByRole('tab').at(-1)!.getBoundingClientRect()
  await expect(Math.abs(bar.right - Number.parseFloat(getComputedStyle(tablist).paddingRight) - last.right)).toBeLessThanOrEqual(1)
}

/** Counts are on exactly when the full row (every count) fits the bar; each marker leads its label. */
async function assertMarkers(section: HTMLElement, expected: { readonly names: readonly string[]; readonly withCounts: readonly string[]; readonly withoutCounts: readonly string[]; readonly leading: readonly string[] }) {
  const root = section.querySelector<HTMLElement>('[data-sidebar-tabs]')!
  const tablist = within(section).getByRole('tablist', { name: 'Sidebar' })
  await expect(within(tablist).getAllByRole('tab').map(tab => tab.getAttribute('aria-label'))).toEqual(expected.names)
  const fits = root.querySelector('[data-sidebar-tabs-measure]')!.getBoundingClientRect().width <= root.getBoundingClientRect().width + 0.5
  await waitFor(() => expect(root).toHaveAttribute('data-sidebar-tab-counts', fits ? 'shown' : 'dropped'))
  await expect(visibleText(tablist)).toEqual(fits ? expected.withCounts : expected.withoutCounts)
  const firsts = within(tablist).getAllByRole('tab').map(tab => tab.querySelector('[data-sidebar-tab-label]')!.parentElement!.firstElementChild)
  await expect(firsts.map(first => first?.getAttribute('data-sidebar-tab-marker') ?? 'label')).toEqual(expected.leading)
  // The marker is decorative and painted by CSS: no DOM text, so a tab's only text is its label, which leads its accessible name.
  for (const marker of tablist.querySelectorAll('[data-sidebar-tab-marker]')) {
    await expect(marker).toHaveAttribute('aria-hidden', 'true')
    await expect(marker.textContent).toBe('')
    await expect(marker.getBoundingClientRect().width).toBeGreaterThan(0)
  }
  for (const tab of within(tablist).getAllByRole('tab')) {
    const label = tab.querySelector('[data-sidebar-tab-label]')!.textContent!
    await expect(tab.textContent!.trim()).toBe(label)
    await expect(tab.getAttribute('aria-label')!.startsWith(label)).toBe(true)
  }
  await assertNoClip(tablist)
}
const plain = { names, withCounts, withoutCounts, leading }

async function keyboardPlay({ canvasElement }: { readonly canvasElement: HTMLElement }) {
  const canvas = within(canvasElement)
  const section = await canvas.findByRole('region', { name: /^Sidebar tabs / })
  const host = section.querySelector<HTMLElement>('[data-sidebar-host]')!
  const tablist = within(section).getByRole('tablist', { name: 'Sidebar' })
  const tabs = within(tablist).getAllByRole('tab')
  await expect(sidebarTabName({ id: 'alerts', label: 'Alerts', needsYouCount: 3, questionCount: 2 })).toBe('Alerts, 2 open questions')
  await assertMarkers(section, plain)
  // Beside the sidebar: the three cards behind Agents ◆3, one per requesting agent, then the mission's five steps.
  const sources = canvas.getByRole('region', { name: 'Marker sources' })
  await expect(within(sources).getAllByRole('list').map(list => list.querySelectorAll('[data-sidebar-item="needs-you"]').length)).toEqual([1, 1, 1, 0])
  await expect(within(within(sources).getAllByRole('list').at(-1)!).getAllByRole('listitem')).toHaveLength(5)
  // Fixed sidebar width at either viewport; only the tab height follows the layout.
  await expect(host.getBoundingClientRect().width).toBe(256)
  await expect(tabs[0]!.getBoundingClientRect().height).toBe(window.innerWidth < 900 ? 44 : 24)

  // Agents is selected; its panel is the only one with content, labelled by its tab; three rows need you, as its marker says; it scrolls inside the frame.
  await expect(tabs[0]).toHaveAttribute('aria-selected', 'true')
  const agentsPanel = within(section).getByRole('tabpanel', { name: names[0] })
  await expect(tabs[0]).toHaveAttribute('aria-controls', agentsPanel.id)
  await expect(within(agentsPanel).getAllByTestId('taste-agent-row')).toHaveLength(4)
  await expect(agentsPanel.querySelectorAll('[data-testid="taste-agent-row"][data-needs-me="true"]')).toHaveLength(3)
  const scroller = agentsPanel.querySelector<HTMLElement>('[data-sidebar-scroller]')!
  await expect(scroller.scrollHeight).toBeGreaterThan(scroller.clientHeight)
  await expect(agentsPanel.getBoundingClientRect().bottom).toBeLessThanOrEqual(host.getBoundingClientRect().bottom)

  // ←/→ move focus and selection together; Home/End reach the ends.
  await userEvent.click(tabs[0]!)
  await userEvent.keyboard('{ArrowRight}')
  await expect(tabs[1]).toHaveFocus()
  await expect(tabs[1]).toHaveAttribute('aria-selected', 'true')
  await expect(within(section).getByTestId('sidebar-tab-selection')).toHaveTextContent('alerts')
  const alertsPanel = within(section).getByRole('tabpanel', { name: names[1] })
  await expect([...alertsPanel.querySelectorAll<HTMLElement>('[data-sidebar-item]')].map(item => item.dataset.sidebarItem)).toEqual(['question', 'needs-you', 'needs-you', 'needs-you'])
  await expect(within(section).queryAllByRole('tabpanel')).toHaveLength(1)
  await userEvent.keyboard('{ArrowRight}')
  await expect(tabs[2]).toHaveFocus()
  const missionsPanel = within(section).getByRole('tabpanel', { name: names[2] })
  await expect([...missionsPanel.querySelectorAll<HTMLElement>('[data-sidebar-item]')].map(item => item.dataset.sidebarItem)).toEqual(['needs-you'])
  await userEvent.keyboard('{End}')
  await expect(tabs[3]).toHaveFocus()
  await expect(within(section).getByTestId('sidebar-tab-selection')).toHaveTextContent('fleet')
  await expect(within(within(section).getByRole('tabpanel', { name: 'Fleet' })).getAllByRole('listitem')).toHaveLength(2)
  await userEvent.keyboard('{ArrowLeft}')
  await expect(tabs[2]).toHaveFocus()
  await expect(within(section).getByTestId('sidebar-tab-selection')).toHaveTextContent('missions')
  await userEvent.keyboard('{Home}')
  await expect(tabs[0]).toHaveFocus()
  await expect(within(section).getByTestId('sidebar-tab-selection')).toHaveTextContent('agents')
  // Selection changes never move a marker or drop a label.
  await assertMarkers(section, plain)
}

/** At the row's exact natural width (equal tracks, each as wide as the widest tab with its count) every count shows; one pixel less drops every count together, and no label truncates. */
async function countFitPlay({ canvasElement }: { readonly canvasElement: HTMLElement }) {
  const section = await within(canvasElement).findByRole('region', { name: /^Sidebar tabs / })
  const host = section.querySelector<HTMLElement>('[data-sidebar-host]')!
  const root = section.querySelector<HTMLElement>('[data-sidebar-tabs]')!
  const tablist = within(section).getByRole('tablist', { name: 'Sidebar' })
  // The replica measures the equal-track row; the host's own frame (its hairline border) sits outside the bar, so it is added back.
  const natural = root.querySelector('[data-sidebar-tabs-measure]')!.getBoundingClientRect().width
  const frame = host.getBoundingClientRect().width - root.getBoundingClientRect().width
  host.style.width = `${Math.ceil(natural + frame)}px`
  await expect(root.getBoundingClientRect().width).toBeGreaterThanOrEqual(natural)
  await waitFor(() => expect(root).toHaveAttribute('data-sidebar-tab-counts', 'shown'))
  await expect(visibleText(tablist)).toEqual(withCounts)
  await assertNoClip(tablist)
  host.style.width = `${Math.floor(natural + frame) - 1}px`
  await expect(root.getBoundingClientRect().width).toBeLessThan(natural)
  await waitFor(() => expect(root).toHaveAttribute('data-sidebar-tab-counts', 'dropped'))
  await expect(visibleText(tablist)).toEqual(withoutCounts)
  await expect(within(tablist).getAllByRole('tab').map(tab => tab.getAttribute('aria-label'))).toEqual(names)
  await assertNoClip(tablist)
  host.style.width = ''
  await assertMarkers(section, plain)
}

const meta = {
  title: 'Fractal UI/Sidebar/Tabs',
  component: SidebarTabsStory,
  parameters: {
    layout: 'fullscreen',
    scenario: { _tag: 'slices', slices: ['roster', 'details', 'attention'], world: 'fleet-mid-refactor' },
    docs: { description: { component: 'Controlled sidebar tabs (React Aria) in one full-width row. Each tab leads with its strongest marker: ? (an open question) beats ◆ (needs you), with a count only above 1, then the status glyph (Fleet ! for an unreachable machine with work); a single space, then the label. Tabs without a marker begin with the label. When the row cannot hold every count, all counts drop and labels never truncate. Names read "Agents, 3 need you".' } },
  },
  args: { scheme: 'dark', attention: 'sidebar-tabs' },
  argTypes: { ...scenarioArgTypes(['roster', 'details', 'attention']), scheme: { options: ['dark', 'light'], control: 'radio' }, both: { control: 'boolean' } },
} satisfies Meta<Args>
export default meta

type Story = StoryObj<Args>
export const Keyboard: Story = { tags: ['critical'], name: 'Keyboard · markers, selection and panel', play: keyboardPlay }
export const CountFit: Story = { tags: ['critical'], name: 'Count fit · counts drop before labels', play: countFitPlay }

/** Both themes with an unreachable machine holding work: ◆3, ?, a single ◆ and Fleet's !. */
export const AllStates: Story = { tags: ['critical'], name: 'All states · ◆3, ?, ◆ and ! in both themes', args: { both: true, roster: 'sidebar-tabs' }, play: async ({ canvasElement }) => {
  const sections = await within(canvasElement).findAllByRole('region', { name: /^Sidebar tabs / })
  await expect(sections.map(section => section.dataset.scheme)).toEqual(['dark', 'light'])
  const faulted = { names: [...names.slice(0, 3), 'Fleet, 1 unreachable with work'], withCounts: [...withCounts.slice(0, 3), '! Fleet'], withoutCounts: [...withoutCounts.slice(0, 3), '! Fleet'], leading: [...leading.slice(0, 3), 'status'] }
  for (const section of sections) {
    await assertMarkers(section, faulted)
    await expect(within(section).getAllByRole('tabpanel')).toHaveLength(1)
  }
} }

const styles = stylex.create({
  canvas: { display: 'flex', flexWrap: 'wrap', alignItems: 'flex-start', gap: s.xl, minHeight: '100vh', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  specimen: { display: 'flex', flexDirection: 'column', gap: s.md, padding: s.md, backgroundColor: surface.canvas, color: ink.fg },
  output: { color: ink.fgMuted },
  sources: { width: g.sidebarDefault, boxSizing: 'border-box' },
  caption: { margin: s.zero, fontWeight: t.weightSemibold },
  // The sidebar keeps its default width at either viewport; CountFit changes it directly. Shorter than four rows, so the panel must scroll.
  sidebar: { display: 'flex', flexDirection: 'column', boxSizing: 'border-box', width: g.sidebarDefault, height: '200px', flexShrink: 0, overflow: 'hidden', backgroundColor: surface.sidebar, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border },
  scroller: { flex: '1 1 0', minHeight: 0, overflowY: 'auto' },
  list: { display: 'flex', flexDirection: 'column', margin: s.zero, padding: s.zero, listStyle: 'none' },
  item: { display: 'flex', flexDirection: 'column', justifyContent: 'center', height: g.sidebarRow, paddingInline: s.row, boxSizing: 'border-box' },
  line: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0, fontSize: t.uiSize, lineHeight: t.uiLeading },
  glyph: { width: g.iconSm, flexShrink: 0, color: ink.fgMuted, textAlign: 'center' },
  fault: { color: tone.dangerFg },
  title: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  question: { flexShrink: 0, color: accent.primary, fontWeight: t.weightSemibold },
  needsYou: { flexShrink: 0, color: tone.attention, fontWeight: t.weightSemibold },
  metric: { marginInlineStart: 'auto', flexShrink: 0, color: ink.fgMuted, fontSize: t.metaSize, fontVariantNumeric: 'tabular-nums' },
  facts: { paddingInlineStart: `calc(${g.iconSm} + ${s.sm})`, color: ink.fgMuted, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
})
