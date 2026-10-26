/** Agent thread tabs: controlled parent/subagent switching with the overflow drawer, at 440 and 1440 in both themes. Synthetic host only. */
import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import * as stylex from '@stylexjs/stylex'
import { Button, Input, TextField } from 'react-aria-components'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { AgentThreadTabs, type AgentThread, type AgentThreadTabsHandle } from './AgentThreadTabs'
import { NarrowLayoutContext } from './NarrowLayout'
import { lightTheme, type Scheme } from '../composition-theme'
import { borderVars as border, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

const threads: readonly AgentThread[] = [
  { id: 'root', label: 'Coordinator' },
  { id: 'layout', label: 'Layout review' },
  { id: 'tokens', label: 'Token audit' },
  { id: 'a11y', label: 'Accessibility pass', readOnly: true },
  { id: 'copy', label: 'Copy edit' },
  { id: 'perf', label: 'Render profiling' },
  { id: 'docs', label: 'Docs update', readOnly: true },
  { id: 'release', label: 'Release notes', readOnly: true },
]
const visibleAt = { 440: 3, 1440: 6 } as const
type Width = keyof typeof visibleAt
interface HarnessProps { readonly scheme: Scheme; readonly width: Width; readonly initialId: string }

/** The host owns selection and the conversation; it renders a composer only for writable threads. */
function ThreadTabsHost({ scheme, width, initialId }: HarnessProps) {
  const [selectedId, setSelectedId] = React.useState(initialId)
  const tabs = React.useRef<AgentThreadTabsHandle>(null)
  const selected = threads.find(thread => thread.id === selectedId)!
  return <NarrowLayoutContext.Provider value={width === 440}>
    <div data-scheme={scheme} data-thread-host {...stylex.props(styles.host, width === 440 ? styles.narrow : styles.wide, scheme === 'light' && lightTheme)}>
      <div {...stylex.props(styles.toolbar)}>
        <Button onPress={() => tabs.current?.focusSelected()} {...stylex.props(styles.button)}>Focus selected thread</Button>
        <output data-testid="thread-selection" {...stylex.props(styles.output)}>{selectedId}</output>
      </div>
      <div {...stylex.props(styles.frame)}>
        <AgentThreadTabs ref={tabs} threads={threads} selectedId={selectedId} onSelectionChange={setSelectedId}>
          <div {...stylex.props(styles.conversation)}>
            <p data-testid="thread-conversation" {...stylex.props(styles.transcript)}>{selected.label} conversation</p>
            {selected.readOnly ? null : <TextField aria-label={`Message ${selected.label}`} {...stylex.props(styles.composer)}><Input {...stylex.props(styles.input)} /></TextField>}
          </div>
        </AgentThreadTabs>
      </div>
    </div>
  </NarrowLayoutContext.Provider>
}

/** The selected tab controls the one conversation panel, which is labelled by that tab. */
async function assertPanelLinked(root: HTMLElement) {
  const tab = within(root).getByRole('tab', { selected: true })
  const panel = within(root).getByRole('tabpanel')
  await waitFor(() => expect(tab.getAttribute('aria-controls')).toBe(panel.id))
  await expect(panel.id).not.toBe('')
  await expect(panel).toHaveAttribute('aria-labelledby', tab.id)
}
const drawerName = 'All agent threads'
const keyboardPlay: NonNullable<Story['play']> = async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  const visible = visibleAt[args.width]
  const tablist = await canvas.findByRole('tablist', { name: 'Agent threads' })
  const tabs = within(tablist).getAllByRole('tab')
  await expect(tabs).toHaveLength(visible)
  const toggle = canvas.getByRole('button', { name: `+${threads.length - visible} more` })
  await expect(toggle).toHaveAttribute('aria-expanded', 'false')

  // The ref handle focuses the selected tab; arrows, Home and End move and select within the strip.
  await userEvent.click(canvas.getByRole('button', { name: 'Focus selected thread' }))
  await expect(tabs[0]).toHaveFocus()
  await userEvent.keyboard('{ArrowRight}')
  await expect(tabs[1]).toHaveFocus()
  await expect(tabs[1]).toHaveAttribute('aria-selected', 'true')
  await expect(canvas.getByTestId('thread-selection')).toHaveTextContent('layout')
  await expect(canvas.getByTestId('thread-conversation')).toHaveTextContent('Layout review conversation')
  await assertPanelLinked(canvasElement)
  await userEvent.keyboard('{End}')
  await expect(tabs[visible - 1]).toHaveFocus()
  await expect(tabs[visible - 1]).toHaveAttribute('aria-selected', 'true')
  await userEvent.keyboard('{Home}')
  await expect(tabs[0]).toHaveFocus()
  await expect(canvas.getByTestId('thread-selection')).toHaveTextContent('root')

  // Tab leaves the strip for "+N more"; the drawer lists every thread and focuses the current one; Escape returns focus.
  await userEvent.tab()
  await expect(toggle).toHaveFocus()
  await userEvent.keyboard('{Enter}')
  const drawer = await canvas.findByRole('complementary', { name: drawerName })
  await expect(toggle).toHaveAttribute('aria-expanded', 'true')
  await expect(toggle).toHaveAttribute('aria-controls', drawer.id)
  await expect(drawer.querySelectorAll('[data-thread-item]')).toHaveLength(threads.length)
  await waitFor(() => expect(drawer.querySelector('[aria-current="true"]')).toHaveFocus())
  const panel = canvasElement.querySelector<HTMLElement>('[data-thread-panel]')!
  const composer = canvas.getByRole('textbox', { name: 'Message Coordinator', hidden: true })
  if (args.width === 440) {
    // The phone drawer covers the conversation. Backward traversal must skip its still-mounted composer.
    await expect(panel).toHaveAttribute('inert')
    await userEvent.tab({ shift: true })
    await expect(within(drawer).getByRole('button', { name: 'Close agent threads' })).toHaveFocus()
    await userEvent.tab({ shift: true })
    await expect(panel.contains(canvasElement.ownerDocument.activeElement)).toBe(false)
    await expect(composer).not.toHaveFocus()
    // userEvent's tab polyfill may try an inert element; the browser must reject that focus too.
    await expect(within(drawer).getByRole('button', { name: 'Close agent threads' })).toHaveFocus()
  } else {
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

  // Choosing a thread beyond the strip closes the drawer and focuses its tab, which takes the strip's last slot.
  const conversation = canvas.getByTestId('thread-conversation')
  await userEvent.keyboard('{Enter}')
  const reopened = await canvas.findByRole('complementary', { name: drawerName })
  await userEvent.click(within(reopened).getByRole('button', { name: /^Release notes/ }))
  await waitFor(() => expect(canvas.queryByRole('complementary', { name: drawerName })).toBeNull())
  const chosen = within(tablist).getByRole('tab', { name: 'Release notes, read only' })
  await waitFor(() => expect(chosen).toHaveFocus())
  await expect(chosen).toHaveAttribute('aria-selected', 'true')
  await expect(within(tablist).getAllByRole('tab')).toHaveLength(visible)
  await expect(canvas.getByTestId('thread-selection')).toHaveTextContent('release')
  await assertPanelLinked(canvasElement)
  // The host's conversation stays mounted across the switch: same element, new content.
  await expect(canvas.getByTestId('thread-conversation')).toBe(conversation)
  await expect(conversation).toHaveTextContent('Release notes conversation')

  // A read-only thread carries the note; the host renders no composer for it.
  await expect(canvas.getByRole('note')).toHaveTextContent('Read only')
  await expect(canvas.queryByRole('textbox')).toBeNull()

  // The Close button also returns focus to "+N more".
  await userEvent.click(canvas.getByRole('button', { name: `+${threads.length - visible} more` }))
  const closable = await canvas.findByRole('complementary', { name: drawerName })
  await userEvent.click(within(closable).getByRole('button', { name: 'Close agent threads' }))
  await waitFor(() => expect(canvas.queryByRole('complementary', { name: drawerName })).toBeNull())
  await expect(canvas.getByRole('button', { name: `+${threads.length - visible} more` })).toHaveFocus()
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
  title: 'Fractal UI/Agent thread tabs',
  component: ThreadTabsHost,
  parameters: {
    scenario: { _tag: 'contract-fixture', reason: 'Host-owned controlled selection over eight synthetic threads exercises strip overflow, the selected-tab guarantee, drawer focus return and read-only notes, which roster slices do not yet express as parent/subagent threads.' },
    layout: 'fullscreen',
    viewport: { options: {
      threads1440: { name: 'Threads desktop', styles: { width: '1440px', height: '900px' }, type: 'desktop' },
      threads440: { name: 'Threads narrow', styles: { width: '440px', height: '900px' }, type: 'mobile' },
    } },
    a11y: { config: { rules: [{ id: 'label-content-name-mismatch', enabled: true }] } },
    docs: { description: { component: 'Controlled parent/subagent thread tabs (React Aria). The strip shows six threads, three in the narrow layout, always including the selected one; "+N more" opens a drawer of every thread. Read-only threads get a note; the host decides whether to render a composer.' } },
  },
  globals: { viewport: { value: 'threads1440', isRotated: false } },
  args: { scheme: 'dark', width: 1440, initialId: 'root' },
  render: args => <main {...stylex.props(styles.canvas, args.scheme === 'light' && lightTheme)}><ThreadTabsHost {...args} /></main>,
} satisfies Meta<typeof ThreadTabsHost>
export default meta
type Story = StoryObj<typeof meta>
export const Wide1440Dark: Story = { play: keyboardPlay }
export const Wide1440Light: Story = { args: { scheme: 'light' }, play: keyboardPlay }
export const Narrow440Dark: Story = { args: { width: 440 }, globals: { viewport: { value: 'threads440', isRotated: false } }, play: keyboardPlay }
export const Narrow440Light: Story = { args: { width: 440, scheme: 'light' }, globals: { viewport: { value: 'threads440', isRotated: false } }, play: keyboardPlay }
export const AllStates: Story = { render: () => <AllStatesMatrix />, play: async ({ canvasElement }) => {
  for (const specimen of specimens) {
    const region = within(canvasElement).getByRole('region', { name: specimen.name })
    const scoped = within(region)
    const tablist = scoped.getByRole('tablist', { name: 'Agent threads' })
    await expect(within(tablist).getAllByRole('tab')).toHaveLength(visibleAt[specimen.width])
    const thread = threads.find(candidate => candidate.id === specimen.initialId)!
    // The selected thread is on the strip even when it sits beyond the visible count.
    const selected = within(tablist).getByRole('tab', { selected: true })
    await expect(selected).toHaveAccessibleName(thread.readOnly ? `${thread.label}, read only` : thread.label)
    await assertPanelLinked(region)
    if (thread.readOnly) {
      await expect(scoped.getByRole('note')).toHaveTextContent(`${thread.label} can be viewed here, not replied to.`)
      await expect(scoped.queryByRole('textbox')).toBeNull()
    } else {
      await expect(scoped.queryByRole('note')).toBeNull()
      await expect(scoped.getByRole('textbox', { name: `Message ${thread.label}` })).toBeVisible()
    }
  }
  // One drawer stays open for the accessibility check of its landmark, list and close control.
  const narrow = within(within(canvasElement).getByRole('region', { name: 'Narrow dark' }))
  await userEvent.click(narrow.getByRole('button', { name: `+${threads.length - visibleAt[440]} more` }))
  const drawer = await narrow.findByRole('complementary', { name: drawerName })
  await waitFor(() => expect(within(drawer).getByRole('button', { name: /^Accessibility pass/ })).toHaveFocus())
} }

const styles = stylex.create({
  canvas: { minHeight: '100vh', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  matrix: { display: 'flex', flexDirection: 'column', gap: s.xl, minHeight: '100vh', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, fontFamily: t.fontSans },
  specimen: { display: 'flex', flexDirection: 'column', gap: s.md, padding: s.md, backgroundColor: surface.canvas, color: ink.fg },
  heading: { margin: s.zero, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  host: { display: 'flex', flexDirection: 'column', gap: s.md, maxWidth: '100%', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
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
