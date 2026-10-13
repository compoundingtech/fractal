import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { SidebarAgentRow, ThreadHeader, assistantDarkTheme, type SidebarAgentRowData } from '@smalltalk/fractal-ui/assistant-ui/shell'
import { colorVars as c, typeVars as t, spaceVars as s } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { lightTheme, type Scheme } from '../../../../packages/fractal-ui/src/assistant-ui/composition-theme.ts'
import { AgentFolders } from '../shell/AgentFolders.tsx'
import type { SidebarNode } from '../folders/sidebarContract.ts'
import { ResponsiveAgentSidebar, sidebarDrawerBreakpoint, sidebarMinimumWidth, useSidebarShortcut } from './ResponsiveAgentSidebar.tsx'

const storyNow = Date.UTC(2026, 9, 8, 12)
const rows: readonly SidebarAgentRowData[] = ['Builder', 'Reviewer'].map(title => ({
  ref: `agent/${title.toLowerCase()}`, id: `agent/${title.toLowerCase()}`, title: `${title} session`, host: 'local', status: 'idle', statusLabel: 'Idle', freshness: 'live',
  children: [], usage: { _tag: 'Known', scope: '24h-root-and-subagents', usd: 4.2, tokens: 2_300_000 }, duration: { _tag: 'Known', scope: '24h-activity-span', ms: 3_600_000 }, lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 180_000 },
}))
const tree: readonly SidebarNode[] = [{ _tag: 'Folder', id: 'product', label: 'Product', collapsed: false, children: rows.map(row => ({ _tag: 'Agent', id: row.ref, subject: row.ref, label: row.title })) }]

/** Fixture data surrounds the same sidebar frame, tree, row and thread header as live. */
const ResponsiveSidebarSurface = ({ viewport, shortcut = true, scheme = 'dark' }: { readonly viewport: number; readonly shortcut?: boolean; readonly scheme?: Scheme }) => {
  const [collapsed, setCollapsed] = React.useState(false)
  const [drawerOpen, setDrawerOpen] = React.useState(false)
  const [selected, setSelected] = React.useState(rows[0]!.ref)
  const maxWidth = Math.max(sidebarMinimumWidth, Math.min(440, viewport - 640))
  const [width, setWidth] = React.useState(Math.min(maxWidth, Math.max(sidebarMinimumWidth, Math.round(viewport * 256 / 1440))))
  const opener = React.useRef<HTMLButtonElement>(null)
  const narrow = viewport < sidebarDrawerBreakpoint
  const toggle = React.useCallback(() => {
    if (narrow) { opener.current?.focus(); setDrawerOpen(value => !value) }
    else setCollapsed(value => !value)
  }, [narrow])
  useSidebarShortcut(shortcut ? toggle : undefined)
  const select = (ref: string) => { setSelected(ref); setDrawerOpen(false) }
  return <main data-sidebar-layout-story="" data-viewport={viewport} data-scheme={scheme} style={{ width: viewport }} {...stylex.props(styles.root, assistantDarkTheme, scheme === 'light' && lightTheme)}>
    <ResponsiveAgentSidebar viewport={viewport} width={width} maxWidth={maxWidth} collapsed={collapsed} drawerOpen={drawerOpen} onDrawerOpenChange={setDrawerOpen} onToggleCollapsed={() => setCollapsed(value => !value)} onWidthChange={setWidth} onWidthCommit={setWidth} onWidthReset={() => setWidth(Math.min(maxWidth, 256))}>
      <Aria.SearchField aria-label="Search agents" {...stylex.props(styles.search)}><Aria.Input placeholder="Search agents" {...stylex.props(styles.input)} /></Aria.SearchField>
      <nav aria-label="Agent roster" {...stylex.props(styles.roster)}>
        <AgentFolders tree={tree} canDrop={() => ({ refused: 'Read-only layout fixture' })} onMove={() => undefined} onCreateFolder={() => undefined} onRenameFolder={() => undefined} onDeleteFolder={() => undefined} onToggleCollapsed={() => undefined} onSelect={select} renderAgent={node => <SidebarAgentRow inTree item={rows.find(row => row.ref === node.id)!} now={storyNow} active={selected === node.id} onOpen={() => select(node.id)} />} />
      </nav>
    </ResponsiveAgentSidebar>
    <section aria-label="Agent workspace" data-sidebar-thread="" {...stylex.props(styles.thread)}>
      <ThreadHeader title={rows.find(row => row.ref === selected)!.title} folder="local" now={storyNow} sidebarCollapsed={narrow || collapsed} sidebarToggleRef={opener} onToggleSidebar={toggle} panelOpen={false} drawerOpen={false} onTogglePanel={() => undefined} onToggleDrawer={() => undefined} />
      <article {...stylex.props(styles.article)}><h1 {...stylex.props(styles.heading)}>Review the sidebar</h1><p>Opening navigation does not move this thread. The roster remains readable in a minimum 240px drawer.</p><Aria.Button {...stylex.props(styles.button)}>Reply to this session</Aria.Button></article>
    </section>
  </main>
}

type LayoutShiftEntry = PerformanceEntry & { readonly value: number; readonly hadRecentInput: boolean; readonly sources: readonly { readonly node: Node | null }[] }
const assertModalIsolation = (dialog: HTMLElement, thread: HTMLElement): void => {
  if (dialog.getAttribute('aria-modal') !== 'true') throw new Error('Navigation dialog must declare aria-modal=true')
  if (thread.closest('[inert], [aria-hidden="true"]') === null) throw new Error('Workspace must be hidden or inert while navigation is open')
  if (dialog.closest('[inert], [aria-hidden="true"]') !== null) throw new Error('Navigation dialog must remain accessible')
}
const drawerPlay = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  await document.fonts.ready
  const root = canvasElement.querySelector<HTMLElement>('[data-sidebar-layout-story]')!
  const thread = root.querySelector<HTMLElement>('[data-sidebar-thread]')!
  const viewport = Number(root.dataset.viewport)
  await expect(getComputedStyle(root).backgroundColor).toBe(root.dataset.scheme === 'light' ? 'rgb(252, 252, 252)' : 'rgb(10, 10, 10)')
  await expect(getComputedStyle(root).color).toBe(root.dataset.scheme === 'light' ? 'rgb(39, 39, 42)' : 'rgb(245, 245, 245)')
  const canvas = within(root)
  const rect = () => { const value = thread.getBoundingClientRect(); return { x: value.x, y: value.y, width: value.width, height: value.height } }
  if (viewport >= sidebarDrawerBreakpoint) {
    const sidebar = canvas.getByRole('complementary', { name: 'Agents' })
    await expect(sidebar.getBoundingClientRect().width).toBeGreaterThanOrEqual(sidebarMinimumWidth)
    await expect(root.querySelector('[role="dialog"]')).toBeNull()
    const splitter = canvas.getByRole('separator', { name: 'Agent sidebar width' })
    splitter.focus()
    await userEvent.keyboard('{Home}')
    await expect(sidebar.getBoundingClientRect().width).toBe(sidebarMinimumWidth)
    root.dataset.drawerMeasurement = JSON.stringify({ viewport, presentation: 'dock', sidebarWidth: sidebar.getBoundingClientRect().width, thread: rect() })
    return
  }
  await expect(canvas.queryByRole('complementary', { name: 'Agents' })).toBeNull()
  // The root caps at the page width (maxWidth: 100%), so At700 in a 440 page measures 440, not the story arg.
  await expect(thread.getBoundingClientRect().width).toBe(root.getBoundingClientRect().width)
  const opener = canvas.getByRole('button', { name: 'Expand sidebar' })
  const before = rect()
  const entries: { readonly value: number; readonly hadRecentInput: boolean }[] = []
  const record = (records: PerformanceEntry[]) => { for (const entry of records as LayoutShiftEntry[]) if (entry.sources.some(source => source.node !== null && thread.contains(source.node))) entries.push({ value: entry.value, hadRecentInput: entry.hadRecentInput }) }
  const observer = new PerformanceObserver(list => record(list.getEntries()))
  observer.observe({ type: 'layout-shift' })
  try {
    await userEvent.click(opener)
    const dialog = await canvas.findByRole('dialog', { name: 'Agent navigation' })
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true))
    await waitFor(() => assertModalIsolation(dialog, thread))
    // Inject the non-modal regression into the same rendered dialog, not a second fixture.
    dialog.removeAttribute('aria-modal')
    try { await expect(() => assertModalIsolation(dialog, thread)).toThrow('Navigation dialog must declare aria-modal=true') }
    finally { dialog.setAttribute('aria-modal', 'true') }
    const hiddenBackground = thread.closest<HTMLElement>('[inert], [aria-hidden="true"]')!
    const inert = hiddenBackground.getAttribute('inert'), ariaHidden = hiddenBackground.getAttribute('aria-hidden')
    hiddenBackground.removeAttribute('inert'); hiddenBackground.removeAttribute('aria-hidden')
    try { await expect(() => assertModalIsolation(dialog, thread)).toThrow('Workspace must be hidden or inert while navigation is open') }
    finally {
      if (inert !== null) hiddenBackground.setAttribute('inert', inert)
      if (ariaHidden !== null) hiddenBackground.setAttribute('aria-hidden', ariaHidden)
    }
    assertModalIsolation(dialog, thread)
    await expect(canvas.getByRole('complementary', { name: 'Agents' }).getBoundingClientRect().width).toBeGreaterThanOrEqual(sidebarMinimumWidth)
    await expect(rect()).toEqual(before)
    await userEvent.keyboard('{Shift>}{Tab}{/Shift}')
    await expect(dialog.contains(document.activeElement)).toBe(true)
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(root.querySelector('[role="dialog"]')).toBeNull())
    await waitFor(() => expect(document.activeElement).toBe(opener))
    await expect(thread.closest('[inert], [aria-hidden="true"]')).toBeNull()
    await userEvent.keyboard('{Control>}b{/Control}')
    const reopened = await canvas.findByRole('dialog', { name: 'Agent navigation' })
    await waitFor(() => expect(reopened.contains(document.activeElement)).toBe(true))
    await waitFor(() => assertModalIsolation(reopened, thread))
    const search = within(reopened).getByRole('searchbox', { name: 'Search agents' })
    await userEvent.click(search)
    await userEvent.keyboard('{Control>}b{/Control}')
    await expect(root.querySelector('[role="dialog"]')).not.toBeNull()
    await userEvent.click(within(reopened).getByRole('row', { name: /Reviewer session/ }))
    await waitFor(() => expect(root.querySelector('[role="dialog"]')).toBeNull())
    await waitFor(() => expect(document.activeElement).toBe(opener))
    await expect(thread.closest('[inert], [aria-hidden="true"]')).toBeNull()
    await userEvent.keyboard('{Meta>}b{/Meta}')
    const commandOpened = await canvas.findByRole('dialog', { name: 'Agent navigation' })
    await waitFor(() => assertModalIsolation(commandOpened, thread))
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(document.activeElement).toBe(opener))
    await expect(root.querySelector('[role="dialog"]')).toBeNull()
    await expect(thread.closest('[inert], [aria-hidden="true"]')).toBeNull()
    await expect(rect()).toEqual(before)
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
    record(observer.takeRecords())
    const threadShift = entries.reduce((total, entry) => total + entry.value, 0)
    await expect(threadShift).toBe(0)
    root.dataset.drawerMeasurement = JSON.stringify({ viewport, presentation: 'drawer', threadBefore: before, threadAfter: rect(), threadRawLayoutShift: threadShift, entries, focusEntered: true, escapeRestoredFocus: true, selectionRestoredFocus: true, shortcutOpened: true, commandShortcutOpened: true, editableShortcutIgnored: true, ariaModal: true, backgroundHidden: true, nonModalControlRejected: true, unmaskedBackgroundControlRejected: true })
  } finally { observer.disconnect() }
}

const meta = { title: 'wf/Sidebar/Responsive navigation', component: ResponsiveSidebarSurface, args: { viewport: 440, scheme: 'dark' }, argTypes: { scheme: { control: 'inline-radio', options: ['dark', 'light'] } }, parameters: { layout: 'fullscreen', docs: { description: { component: 'Navigation has a 240px floor. Below 900px it is a closed-by-default React Aria modal drawer: aria-modal=true and the background workspace is inert or hidden from assistive technology until dismissal. The thread keeps its full width. Escape or selection closes it and restores opener focus. Cmd/Ctrl+B opens navigation outside editable/terminal input. Opening the drawer must produce zero raw thread layout shift, including input-associated entries. Dark and light plays assert the surface colours and reject injected non-modal and unmasked-background regressions.' } } } } satisfies Meta<typeof ResponsiveSidebarSurface>
export default meta
type Story = StoryObj<typeof meta>
export const At440: Story = { args: { viewport: 440 }, play: drawerPlay }
export const At700: Story = { args: { viewport: 700 }, play: drawerPlay }
export const At900: Story = { args: { viewport: 900 }, play: drawerPlay }
export const At1440: Story = { args: { viewport: 1440 }, play: drawerPlay }
export const LightAt440: Story = { args: { viewport: 440, scheme: 'light' }, play: drawerPlay }
export const LightAt1440: Story = { args: { viewport: 1440, scheme: 'light' }, play: drawerPlay }
export const AllStates: Story = { render: args => <div {...stylex.props(styles.matrix)}>{[440, 700, 900, 1440].map(viewport => <section key={viewport}><h2>{viewport}px</h2><ResponsiveSidebarSurface viewport={viewport} scheme={args.scheme} shortcut={false} /></section>)}</div> }

const styles = stylex.create({
  root: { display: 'flex', height: 'calc(100dvh - var(--wf-devbar-space, 0px))', maxWidth: '100%', overflow: 'hidden', backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  thread: { display: 'flex', flexDirection: 'column', flexGrow: 1, minWidth: 0, minHeight: 0 },
  article: { padding: s.lg, maxWidth: 720 },
  heading: { fontSize: t.uiSize, marginBlock: s.md },
  search: { margin: s.md, padding: s.md, border: `1px solid ${c.border}`, borderRadius: 8 },
  input: { width: '100%', minWidth: 0, borderWidth: 0, outline: 'none', backgroundColor: 'transparent', color: c.fg },
  roster: { display: 'flex', flexDirection: 'column', padding: s.md, flexGrow: 1, minHeight: 0, overflow: 'hidden' },
  button: { backgroundColor: c.raised, color: c.fg, border: `1px solid ${c.borderStrong}`, borderRadius: 6, padding: s.md },
  matrix: { display: 'grid', gap: s.lg, overflow: 'auto', padding: s.lg },
})
