/** Agent thread tabs: Main and its subagent threads, measured to the strip's width, with the overflow drawer, at 440 and 1440 in both themes. Synthetic host only. */
import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import * as stylex from '@stylexjs/stylex'
import { Button, Input, TextField } from 'react-aria-components'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { AgentThreadTabs, type AgentThread, type AgentThreadTabsHandle } from './AgentThreadTabs'
import { NarrowLayoutContext } from './NarrowLayout'
import { lightTheme, type Scheme } from '../composition-theme'
import { borderVars as border, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

/** The parent host labels its own thread "Main"; subagent threads are read only. */
const threads: readonly AgentThread[] = [
  { id: 'root', label: 'Main' },
  { id: 'layout', label: 'Layout review', readOnly: true },
  { id: 'tokens', label: 'Token audit', readOnly: true },
  { id: 'a11y', label: 'Accessibility pass', readOnly: true },
  { id: 'copy', label: 'Copy edit', readOnly: true },
  { id: 'perf', label: 'Render profiling', readOnly: true },
  { id: 'docs', label: 'Docs update', readOnly: true },
  { id: 'release', label: 'Release notes', readOnly: true },
]
type Width = 440 | 1440
interface HarnessProps {
  readonly scheme: Scheme
  /** Layout context: 440 is the narrow (phone) layout. */
  readonly width: Width
  /** Pane width when it differs from the layout, e.g. a narrow pane in the wide layout. */
  readonly pane?: Width
  /** How many threads exist so far, parent first; the host can add the rest one at a time. */
  readonly threadCount?: number
  readonly initialId: string
}

/** The host owns selection and the conversation; it renders a composer only for writable threads. */
function ThreadTabsHost({ scheme, width, pane = width, threadCount = threads.length, initialId }: HarnessProps) {
  const [selectedId, setSelectedId] = React.useState(initialId)
  const [count, setCount] = React.useState(threadCount)
  const tabs = React.useRef<AgentThreadTabsHandle>(null)
  const present = threads.slice(0, count)
  const selected = present.find(thread => thread.id === selectedId) ?? present[0]!
  return <NarrowLayoutContext.Provider value={width === 440}>
    <div data-scheme={scheme} data-thread-host {...stylex.props(styles.host, pane === 440 ? styles.narrow : styles.wide, scheme === 'light' && lightTheme)}>
      <div {...stylex.props(styles.toolbar)}>
        <Button onPress={() => tabs.current?.focusSelected()} {...stylex.props(styles.button)}>Focus selected thread</Button>
        {count < threads.length ? <Button onPress={() => setCount(count + 1)} {...stylex.props(styles.button)}>Add subagent</Button> : null}
        <output data-testid="thread-selection" {...stylex.props(styles.output)}>{selected.id}</output>
      </div>
      <div {...stylex.props(styles.frame)}>
        <AgentThreadTabs ref={tabs} threads={present} selectedId={selected.id} onSelectionChange={setSelectedId}>
          <div {...stylex.props(styles.conversation)}>
            <p data-testid="thread-conversation" {...stylex.props(styles.transcript)}>{selected.label} conversation</p>
            {selected.readOnly ? null : <TextField aria-label={`Message ${selected.label}`} {...stylex.props(styles.composer)}><Input {...stylex.props(styles.input)} /></TextField>}
          </div>
        </AgentThreadTabs>
      </div>
    </div>
  </NarrowLayoutContext.Provider>
}

const stripName = 'Subagents'
const drawerName = 'Subagent threads'
const moreName = /^\+\d+ more$/
const noticeText = 'Subagent thread · read only — reply in Main'

/** Waits for the strip's first width measurement and returns the component root. */
async function measuredRoot(root: HTMLElement) {
  await waitFor(() => expect(root.querySelector('[data-agent-thread-tabs][data-thread-strip-measured]')).not.toBeNull())
  return root.querySelector<HTMLElement>('[data-agent-thread-tabs]')!
}

/** The strip keeps thread order (Main first when it fits) and always holds the selected thread. */
async function assertStripOrder(tablist: HTMLElement) {
  const ids = within(tablist).getAllByRole('tab').map(tab => tab.dataset.threadId!)
  await expect(ids).toEqual(threads.map(thread => thread.id).filter(id => ids.includes(id)))
  await expect(within(tablist).getByRole('tab', { selected: true })).toBeVisible()
  const tabs = within(tablist).getAllByRole('tab')
  const strip = tablist.closest<HTMLElement>('[data-agent-thread-strip]')!
  const toggle = strip.querySelector<HTMLElement>('[data-thread-drawer-toggle]')
  const right = toggle?.getBoundingClientRect().left ?? strip.getBoundingClientRect().right
  for (const tab of tabs) await expect(tab.getBoundingClientRect().right).toBeLessThanOrEqual(right + 1)
}

/** The selected tab controls the one conversation panel, which is labelled by that tab. */
async function assertPanelLinked(root: HTMLElement) {
  const tab = within(root).getByRole('tab', { selected: true })
  const panel = within(root).getByRole('tabpanel')
  await waitFor(() => expect(tab.getAttribute('aria-controls')).toBe(panel.id))
  await expect(panel.id).not.toBe('')
  await expect(panel).toHaveAttribute('aria-labelledby', tab.id)
}

/** A read-only subagent thread ends with the notice where the composer would be: after the transcript, at the panel's bottom edge. */
async function assertNoticeAtEnd(root: HTMLElement) {
  const panel = root.querySelector<HTMLElement>('[data-thread-panel]')!
  const note = within(panel).getByRole('note')
  const conversation = within(panel).getByTestId('thread-conversation')
  await expect(note).toHaveAttribute('data-thread-read-only', panel.dataset.threadPanel)
  await expect(note).toHaveTextContent(noticeText)
  await expect(panel.lastElementChild).toBe(note)
  await expect(conversation.compareDocumentPosition(note) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  await expect(note.getBoundingClientRect().top).toBeGreaterThanOrEqual(conversation.getBoundingClientRect().bottom - 1)
  await expect(Math.abs(note.getBoundingClientRect().bottom - panel.getBoundingClientRect().bottom)).toBeLessThanOrEqual(1)
  await expect(within(note).getByRole('button', { name: 'Main' })).toHaveAttribute('data-thread-reply-in-parent')
  await expect(within(root).queryByRole('textbox')).toBeNull()
}

const keyboardPlay: NonNullable<Story['play']> = async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  const overflows = (args.pane ?? args.width) === 440
  await measuredRoot(canvasElement)
  const tablist = canvas.getByRole('tablist', { name: stripName })
  const tabs = within(tablist).getAllByRole('tab')
  const visible = tabs.length
  await assertStripOrder(tablist)
  await expect(tabs[0]).toHaveAccessibleName('Main')
  await expect(tabs[0]).toHaveAttribute('data-thread-role', 'parent')
  if (overflows) {
    // Only the tabs that fit beside "+N more" show; the count is measured, not fixed.
    await expect(visible).toBeGreaterThan(1)
    await expect(visible).toBeLessThan(threads.length)
    await expect(canvas.getByRole('button', { name: `+${threads.length - visible} more` })).toHaveAttribute('aria-expanded', 'false')
  } else {
    // Eight ordinary threads all fit a wide pane: no overflow toggle.
    await expect(visible).toBe(threads.length)
    await expect(canvas.queryByRole('button', { name: moreName })).toBeNull()
  }

  // The ref handle focuses the selected tab; arrows, Home and End move and select within the strip.
  await userEvent.click(canvas.getByRole('button', { name: 'Focus selected thread' }))
  await expect(tabs[0]).toHaveFocus()
  await userEvent.keyboard('{ArrowRight}')
  await expect(tabs[1]).toHaveFocus()
  await expect(tabs[1]).toHaveAttribute('aria-selected', 'true')
  await expect(canvas.getByTestId('thread-selection')).toHaveTextContent('layout')
  await expect(canvas.getByTestId('thread-conversation')).toHaveTextContent('Layout review conversation')
  await assertPanelLinked(canvasElement)
  await assertNoticeAtEnd(canvasElement)
  await userEvent.keyboard('{End}')
  await expect(tabs[visible - 1]).toHaveFocus()
  await expect(tabs[visible - 1]).toHaveAttribute('aria-selected', 'true')
  await userEvent.keyboard('{Home}')
  await expect(tabs[0]).toHaveFocus()
  await expect(canvas.getByTestId('thread-selection')).toHaveTextContent('root')
  await expect(canvas.queryByRole('note')).toBeNull()
  if (!overflows) return

  // Tab leaves the strip for "+N more"; the drawer groups Main and its subagents and focuses the current one; Escape returns focus.
  const toggle = canvas.getByRole('button', { name: `+${threads.length - visible} more` })
  await userEvent.tab()
  await expect(toggle).toHaveFocus()
  await userEvent.keyboard('{Enter}')
  const drawer = await canvas.findByRole('complementary', { name: drawerName })
  await expect(toggle).toHaveAttribute('aria-expanded', 'true')
  await expect(toggle).toHaveAttribute('aria-controls', drawer.id)
  await expect(drawer.querySelectorAll('[data-thread-group="parent"] [data-thread-item]')).toHaveLength(1)
  const subagentList = within(drawer).getByRole('list', { name: /^Subagents/ })
  await expect(subagentList.querySelectorAll('[data-thread-item][data-thread-role="subagent"]')).toHaveLength(threads.length - 1)
  // One "Read only" on the group, none per row.
  await expect(within(drawer).getAllByText('Read only')).toHaveLength(1)
  await waitFor(() => expect(drawer.querySelector('[aria-current="true"]')).toHaveFocus())
  const panel = canvasElement.querySelector<HTMLElement>('[data-thread-panel]')!
  const composer = canvas.getByRole('textbox', { name: 'Message Main', hidden: true })
  const close = within(drawer).getByRole('button', { name: 'Close subagent threads' })
  if (args.width === 440) {
    // The phone drawer covers the conversation. Backward traversal must skip its still-mounted composer.
    await expect(panel).toHaveAttribute('inert')
    await expect(composer.closest('[inert]')).toBe(panel)
    await userEvent.tab({ shift: true })
    await expect(close).toHaveFocus()
    await userEvent.tab({ shift: true })
    await expect(panel.contains(canvasElement.ownerDocument.activeElement)).toBe(false)
    await expect(composer).not.toHaveFocus()
    // userEvent's tab polyfill may try an inert element; the browser must reject that focus too, as it does a direct focus().
    await expect(close).toHaveFocus()
    composer.focus()
    await expect(composer).not.toHaveFocus()
    await expect(close).toHaveFocus()
  } else {
    // Beside a wide-layout drawer the conversation stays interactive, even in a narrow pane.
    await expect(panel).not.toHaveAttribute('inert')
  }
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(canvas.queryByRole('complementary', { name: drawerName })).toBeNull())
  await expect(toggle).toHaveFocus()
  await expect(toggle).toHaveAttribute('aria-expanded', 'false')
  await expect(panel).not.toHaveAttribute('inert')
  // Closing the drawer restores the parent composer to the keyboard order without remounting it.
  await userEvent.tab()
  await expect(composer).toHaveFocus()
  await userEvent.tab({ shift: true })
  await expect(toggle).toHaveFocus()

  // Choosing a thread beyond the strip closes the drawer and focuses its tab, which takes the strip's last slots.
  const conversation = canvas.getByTestId('thread-conversation')
  await userEvent.keyboard('{Enter}')
  const reopened = await canvas.findByRole('complementary', { name: drawerName })
  await userEvent.click(within(reopened).getByRole('button', { name: 'Release notes' }))
  await waitFor(() => expect(canvas.queryByRole('complementary', { name: drawerName })).toBeNull())
  const chosen = within(tablist).getByRole('tab', { name: 'Release notes, read only' })
  await waitFor(() => expect(chosen).toHaveFocus())
  await expect(chosen).toHaveAttribute('aria-selected', 'true')
  const after = within(tablist).getAllByRole('tab')
  await expect(after.at(-1)).toBe(chosen)
  await expect(after.length).toBeLessThan(threads.length)
  await assertStripOrder(tablist)
  await expect(canvas.getByTestId('thread-selection')).toHaveTextContent('release')
  await assertPanelLinked(canvasElement)
  // The host's conversation stays mounted across the switch: same element, new content.
  await expect(canvas.getByTestId('thread-conversation')).toBe(conversation)
  await expect(conversation).toHaveTextContent('Release notes conversation')
  await assertNoticeAtEnd(canvasElement)

  // The Close button also returns focus to "+N more".
  const moreAfter = canvas.getByRole('button', { name: `+${threads.length - after.length} more` })
  await userEvent.click(moreAfter)
  const closable = await canvas.findByRole('complementary', { name: drawerName })
  await userEvent.click(within(closable).getByRole('button', { name: 'Close subagent threads' }))
  await waitFor(() => expect(canvas.queryByRole('complementary', { name: drawerName })).toBeNull())
  await expect(moreAfter).toHaveFocus()
}

const specimens = [
  { name: 'Wide dark', scheme: 'dark', width: 1440, initialId: 'root' },
  { name: 'Wide light', scheme: 'light', width: 1440, initialId: 'docs' },
  { name: 'Narrow dark', scheme: 'dark', width: 440, initialId: 'a11y' },
  { name: 'Narrow light', scheme: 'light', width: 440, initialId: 'layout' },
] as const satisfies readonly (HarnessProps & { readonly name: string })[]

function AllStatesMatrix() {
  return <main aria-label="Agent thread tab states" {...stylex.props(styles.matrix)}>
    {specimens.map(({ name, ...props }) => <section key={name} aria-label={name} data-specimen={name} {...stylex.props(styles.specimen, props.scheme === 'light' && lightTheme)}>
      <h2 {...stylex.props(styles.heading)}>{name}</h2>
      <ThreadTabsHost {...props} />
    </section>)}
  </main>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Agent thread tabs',
  component: ThreadTabsHost,
  parameters: {
    scenario: { _tag: 'contract-fixture', reason: 'Host-owned controlled selection over Main and seven synthetic read-only subagent threads exercises measured strip overflow, the selected-tab guarantee, drawer focus return and the read-only notice, which roster slices do not yet express as parent/subagent threads.' },
    layout: 'fullscreen',
    viewport: { options: {
      threads1440: { name: 'Threads desktop', styles: { width: '1440px', height: '900px' }, type: 'desktop' },
      threads440: { name: 'Threads narrow', styles: { width: '440px', height: '900px' }, type: 'mobile' },
    } },
    a11y: { config: { rules: [{ id: 'label-content-name-mismatch', enabled: true }] } },
    docs: { description: { component: 'Controlled Main/subagent thread tabs (React Aria), shown once Main has a subagent. The strip measures its own width and shows every tab that fits, always including the selected one, in thread order; "+N more" opens a drawer grouping Main and its subagents. A read-only subagent thread ends with "Subagent thread · read only — reply in Main" where the composer would be; the host decides whether to render a composer.' } },
  },
  globals: { viewport: { value: 'threads1440', isRotated: false } },
  args: { scheme: 'dark', width: 1440, initialId: 'root' },
  render: args => <main {...stylex.props(styles.canvas, args.scheme === 'light' && lightTheme)}><ThreadTabsHost {...args} /></main>,
} satisfies Meta<typeof ThreadTabsHost>
export default meta

// Reviewed scenario exemptions by story export and mode, co-located with the synthetic host contract.
export const scenarioExemptions: readonly string[] = [
  'AllStates:contract-fixture',
  'Narrow440Dark:contract-fixture',
  'Narrow440Light:contract-fixture',
  'ReadOnlyNotice:contract-fixture',
  'ResizeFit:contract-fixture',
  'SingleParent:contract-fixture',
  'Wide1440Dark:contract-fixture',
  'Wide1440Light:contract-fixture',
  'WideLayoutNarrowPane:contract-fixture',
]
type Story = StoryObj<typeof meta>
export const Wide1440Dark: Story = { play: keyboardPlay }
export const Wide1440Light: Story = { args: { scheme: 'light' }, play: keyboardPlay }
export const Narrow440Dark: Story = { args: { width: 440 }, globals: { viewport: { value: 'threads440', isRotated: false } }, play: keyboardPlay }
export const Narrow440Light: Story = { args: { width: 440, scheme: 'light' }, globals: { viewport: { value: 'threads440', isRotated: false } }, play: keyboardPlay }
/** A narrow pane inside the wide layout overflows by measurement; its drawer sits beside the conversation. */
export const WideLayoutNarrowPane: Story = { args: { pane: 440 }, play: keyboardPlay }

/** Main alone has no strip; the first subagent brings the strip without remounting the conversation or losing a draft. */
export const SingleParent: Story = { args: { threadCount: 1 }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const root = canvasElement.querySelector<HTMLElement>('[data-agent-thread-tabs]')!
  await expect(root).toHaveAttribute('data-thread-count', '1')
  await expect(canvas.queryByRole('tablist')).toBeNull()
  await expect(canvasElement.querySelector('[data-agent-thread-strip]')).toBeNull()
  await expect(canvas.queryByRole('tabpanel')).toBeNull()
  await expect(canvas.queryByRole('button', { name: moreName })).toBeNull()
  await expect(canvas.queryByRole('note')).toBeNull()
  const conversation = canvas.getByTestId('thread-conversation')
  const composer = canvas.getByRole('textbox', { name: 'Message Main' })
  await userEvent.type(composer, 'draft')
  // Without a strip there is no tab to focus.
  const focusButton = canvas.getByRole('button', { name: 'Focus selected thread' })
  await userEvent.click(focusButton)
  await expect(focusButton).toHaveFocus()

  await userEvent.click(canvas.getByRole('button', { name: 'Add subagent' }))
  const tablist = await canvas.findByRole('tablist', { name: stripName })
  await measuredRoot(canvasElement)
  await expect(within(tablist).getAllByRole('tab').map(tab => tab.textContent)).toEqual(['Main', 'Layout review'])
  await expect(within(tablist).getByRole('tab', { selected: true })).toHaveAccessibleName('Main')
  await assertPanelLinked(canvasElement)
  await expect(canvas.getByTestId('thread-conversation')).toBe(conversation)
  await expect(canvas.getByRole('textbox', { name: 'Message Main' })).toBe(composer)
  await expect(composer).toHaveValue('draft')
} }

/** The strip re-measures as its pane resizes: 1440 fits all eight tabs, 440 overflows, and the selected tab stays shown. */
export const ResizeFit: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const host = canvasElement.querySelector<HTMLElement>('[data-thread-host]')!
  const root = await measuredRoot(canvasElement)
  const tablist = canvas.getByRole('tablist', { name: stripName })
  await expect(within(tablist).getAllByRole('tab')).toHaveLength(threads.length)
  await userEvent.click(within(tablist).getByRole('tab', { name: 'Release notes, read only' }))
  await expect(canvas.getByTestId('thread-selection')).toHaveTextContent('release')

  host.style.width = '440px'
  await waitFor(() => expect(canvas.getByRole('button', { name: moreName })).toBeVisible())
  const narrowCount = within(tablist).getAllByRole('tab').length
  await expect(narrowCount).toBeLessThan(threads.length)
  await expect(root).toHaveAttribute('data-thread-hidden', String(threads.length - narrowCount))
  const selected = within(tablist).getByRole('tab', { selected: true })
  await expect(selected).toHaveAccessibleName('Release notes, read only')
  await expect(within(tablist).getAllByRole('tab').at(-1)).toBe(selected)
  await assertStripOrder(tablist)
  await assertPanelLinked(canvasElement)
  // Settled: the measurement does not feed back into another resize.
  const settled = root.dataset.threadShown
  for (let frame = 0; frame < 4; frame++) {
    await new Promise<number>(resolve => requestAnimationFrame(resolve))
  }
  await expect(root.dataset.threadShown).toBe(settled)

  host.style.width = '1440px'
  await waitFor(() => expect(canvas.queryByRole('button', { name: moreName })).toBeNull())
  await expect(within(tablist).getAllByRole('tab')).toHaveLength(threads.length)
  await assertStripOrder(tablist)

  host.style.width = '440px'
  await waitFor(() => expect(within(tablist).getAllByRole('tab')).toHaveLength(narrowCount))
  await expect(within(tablist).getByRole('tab', { selected: true })).toHaveAccessibleName('Release notes, read only')
  await assertPanelLinked(canvasElement)
} }

/** The notice replaces the composer at the bottom of a read-only subagent thread; its Main button returns to the parent tab. */
export const ReadOnlyNotice: Story = { args: { initialId: 'docs' }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await measuredRoot(canvasElement)
  await assertNoticeAtEnd(canvasElement)
  const conversation = canvas.getByTestId('thread-conversation')
  await userEvent.click(within(canvas.getByRole('note')).getByRole('button', { name: 'Main' }))
  const main = within(canvas.getByRole('tablist', { name: stripName })).getByRole('tab', { name: 'Main' })
  await waitFor(() => expect(main).toHaveFocus())
  await expect(main).toHaveAttribute('aria-selected', 'true')
  await expect(canvas.getByTestId('thread-selection')).toHaveTextContent('root')
  await expect(canvas.queryByRole('note')).toBeNull()
  await expect(canvas.getByRole('textbox', { name: 'Message Main' })).toBeVisible()
  await expect(canvas.getByTestId('thread-conversation')).toBe(conversation)
  await assertPanelLinked(canvasElement)
} }

export const AllStates: Story = { render: () => <AllStatesMatrix />, play: async ({ canvasElement }) => {
  for (const specimen of specimens) {
    const region = within(canvasElement).getByRole('region', { name: specimen.name })
    const scoped = within(region)
    await measuredRoot(region)
    const tablist = scoped.getByRole('tablist', { name: stripName })
    const count = within(tablist).getAllByRole('tab').length
    if (specimen.width === 1440) await expect(count).toBe(threads.length)
    else await expect(count).toBeLessThan(threads.length)
    await assertStripOrder(tablist)
    const thread = threads.find(candidate => candidate.id === specimen.initialId)!
    // The selected thread is on the strip even when it sits beyond the tabs that fit.
    await expect(within(tablist).getByRole('tab', { selected: true })).toHaveAccessibleName(thread.readOnly ? `${thread.label}, read only` : thread.label)
    await assertPanelLinked(region)
    if (thread.readOnly) await assertNoticeAtEnd(region)
    else {
      await expect(scoped.queryByRole('note')).toBeNull()
      await expect(scoped.getByRole('textbox', { name: `Message ${thread.label}` })).toBeVisible()
    }
  }
  // One drawer stays open for the accessibility check of its landmark, groups and close control.
  const narrow = within(within(canvasElement).getByRole('region', { name: 'Narrow dark' }))
  await userEvent.click(narrow.getByRole('button', { name: moreName }))
  const drawer = await narrow.findByRole('complementary', { name: drawerName })
  await waitFor(() => expect(within(drawer).getByRole('button', { name: 'Accessibility pass' })).toHaveFocus())
} }

const styles = stylex.create({
  canvas: { minHeight: '100vh', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  matrix: { display: 'flex', flexDirection: 'column', gap: s.xl, minHeight: '100vh', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, fontFamily: t.fontSans },
  specimen: { display: 'flex', flexDirection: 'column', gap: s.md, padding: s.md, backgroundColor: surface.canvas, color: ink.fg },
  heading: { margin: s.zero, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  // Like the composer width-budget frames, specimens own their width even in a narrower runner viewport.
  // ResizeFit changes this width directly; a viewport max-width would silently cap its wide phase.
  host: { display: 'flex', flexDirection: 'column', gap: s.md, flexShrink: 0, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  wide: { width: g.polishWide }, narrow: { width: g.polishNarrow },
  toolbar: { display: 'flex', alignItems: 'center', gap: s.md },
  button: { minHeight: g.controlMd, paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.control, backgroundColor: surface.controlFill, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  output: { color: ink.fgMuted, fontVariantNumeric: 'tabular-nums' },
  frame: { display: 'flex', flexDirection: 'column', height: '360px', overflow: 'hidden', borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md },
  conversation: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minHeight: 0, padding: s.lg, gap: s.lg },
  transcript: { flex: '1 1 0', margin: s.zero, color: ink.fgSoft },
  composer: { display: 'flex' },
  input: { flex: '1 1 0', minWidth: 0, minHeight: g.controlLg, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.control, backgroundColor: surface.message, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
})
