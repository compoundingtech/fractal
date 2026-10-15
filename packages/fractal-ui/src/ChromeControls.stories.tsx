import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { ChromeControl } from './assistant-ui/composition/ChromeControl'
import { ThreadHeader } from './assistant-ui/composition/Shell'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

/**
 * The chrome's quiet icon controls in place: the sidebar header and footer, and the thread header with the
 * agent, resources and conversation controls beside the terminal and panel toggles (panel open, so pressed).
 * Below 640px the sidebar strip stacks above the header.
 */
function ChromeControlsStory({ scheme = 'dark' }: { scheme?: Scheme }) {
  const [collapsed, setCollapsed] = React.useState(false)
  const [panelOpen, setPanelOpen] = React.useState(true)
  const [drawerOpen, setDrawerOpen] = React.useState(false)
  const [needsMe, setNeedsMe] = React.useState(false)
  return <main data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <aside aria-label="Agents" {...stylex.props(styles.sidebar)}>
      <header {...stylex.props(styles.band)}>
        <ChromeControl icon={collapsed ? 'chevron-right' : 'chevrons-left'} label={collapsed ? 'Expand agents' : 'Collapse agents'} onPress={() => setCollapsed(value => !value)} />
        <strong>Fractal</strong>
        <span {...stylex.props(styles.push)} />
        <ChromeControl icon="bell" label="Needs me only" pressed={needsMe} onPress={() => setNeedsMe(value => !value)} />
        <ChromeControl icon="filter" label="Filter agents · 1 active" active />
        <ChromeControl icon="sort" label="Sort agents · Manual" />
      </header>
      <footer {...stylex.props(styles.band)}>
        <ChromeControl icon="swap" label="Toggle color scheme" tooltipPlacement="top" />
        <ChromeControl icon="checklist" label="Missions" tooltipPlacement="top" />
        <ChromeControl icon="gauge" label="Usage" tooltipPlacement="top" />
      </footer>
    </aside>
    <section aria-label="Agent workspace" {...stylex.props(styles.workspace)}>
      <ThreadHeader folder="dev5" title="fractal-web" status="working" statusLabel="working" panelOpen={panelOpen} drawerOpen={drawerOpen} onTogglePanel={() => setPanelOpen(value => !value)} onToggleDrawer={() => setDrawerOpen(value => !value)} terminalAvailable nativeActions={
        <div {...stylex.props(styles.actions)}>
          <ChromeControl icon="person" label="Actions for fractal-web" />
          <ChromeControl icon="folder" label="Resources" />
          <ChromeControl icon="gear" label="Conversation menu" />
        </div>
      } />
    </section>
  </main>
}

const meta = {
  title: 'Fractal UI/Chrome controls',
  component: ChromeControlsStory,
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
} satisfies Meta<typeof ChromeControlsStory>
export default meta
type Story = StoryObj<typeof meta>

const header = (canvasElement: HTMLElement) => within(canvasElement).getByTestId('thread-header')
const headerNames = ['Actions for fractal-web', 'Resources', 'Conversation menu', 'Toggle terminal drawer', 'Toggle right panel']

/** Every chrome control is a 28px icon button named by its accessible name; open toggles and set filters keep the fill. */
export const QuietIcons: Story = { play: async ({ canvasElement }) => {
  const buttons = within(header(canvasElement)).getAllByRole('button')
  await expect(buttons.map(button => button.getAttribute('aria-label'))).toEqual(headerNames)
  for (const button of within(canvasElement).getAllByRole('button')) {
    await expect(button.textContent?.trim()).toBe('')
    await expect(button.querySelectorAll('svg')).toHaveLength(1)
    await expect(button.getBoundingClientRect().height).toBe(28)
  }
  await expect(within(header(canvasElement)).getByRole('button', { name: 'Toggle right panel' })).toHaveAttribute('aria-pressed', 'true')
  await expect(within(header(canvasElement)).getByRole('button', { name: 'Toggle terminal drawer' })).toHaveAttribute('aria-pressed', 'false')
  const sidebar = within(within(canvasElement).getByRole('complementary', { name: 'Agents' }))
  await expect(sidebar.getByRole('button', { name: 'Filter agents · 1 active' })).not.toHaveAttribute('aria-pressed')
  await expect(sidebar.getAllByRole('button').map(button => button.getAttribute('aria-label'))).toEqual(['Collapse agents', 'Needs me only', 'Filter agents · 1 active', 'Sort agents · Manual', 'Toggle color scheme', 'Missions', 'Usage'])
} }
export const QuietIconsLight: Story = { ...QuietIcons, args: { scheme: 'light' } }

/** Hover names Resources in the kit tooltip; Tab from Agent lands on Resources with the focus ring and the same tooltip; a toggle press keeps its fill. */
export const QuietIconsFlow: Story = { play: async ({ canvasElement }) => {
  const scope = within(header(canvasElement))
  const resources = scope.getByRole('button', { name: 'Resources' })
  // The pointer crosses the header on its way to a control; the page's very first hover opens no tooltip.
  await userEvent.hover(header(canvasElement))
  await userEvent.hover(resources)
  await waitFor(() => expect(within(document.body).getByRole('tooltip')).toHaveTextContent('Resources'))
  await userEvent.unhover(resources)
  await waitFor(() => expect(within(document.body).queryByRole('tooltip')).toBeNull())
  scope.getByRole('button', { name: 'Actions for fractal-web' }).focus()
  await userEvent.tab()
  await expect(resources).toHaveFocus()
  await expect(resources).toHaveAttribute('data-focus-visible', 'true')
  await waitFor(() => expect(getComputedStyle(resources).outlineStyle).toBe('solid'))
  await waitFor(() => expect(within(document.body).getByRole('tooltip')).toHaveTextContent('Resources'))
  await userEvent.keyboard('{Escape}')
  const terminal = scope.getByRole('button', { name: 'Toggle terminal drawer' })
  await userEvent.click(terminal)
  await expect(terminal).toHaveAttribute('aria-pressed', 'true')
} }

const styles = stylex.create({
  root: { display: 'flex', flexDirection: { default: 'column', '@media (min-width: 640px)': 'row' }, minHeight: '100vh', boxSizing: 'border-box', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  sidebar: { display: 'flex', flexDirection: 'column', justifyContent: 'space-between', flexShrink: 0, width: { default: 'auto', '@media (min-width: 640px)': g.sidebarDefault }, backgroundColor: surface.sidebar },
  band: { height: g.footer, display: 'flex', alignItems: 'center', gap: s.xs, paddingInline: s.lg },
  push: { flexGrow: 1 },
  workspace: { display: 'flex', flexDirection: 'column', flexGrow: 1, minWidth: 0 },
  actions: { display: 'flex', alignItems: 'center', gap: s.xs },
})
