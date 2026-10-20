import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { ThreadHeader } from './assistant-ui/composition/Shell'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { colorVars as c, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const terminalReason = 'The terminal opens once this agent is running on a host.'
function ThreadHeaderStory({ scheme = 'dark', terminalDisabledReason, narrow = false, laneWidth }: { scheme?: Scheme; terminalDisabledReason?: string; narrow?: boolean; laneWidth?: number }) {
  return <main data-scheme={scheme} style={laneWidth === undefined ? undefined : { width: laneWidth }} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    {narrow
      ? <div data-testid="phone-frame" {...stylex.props(styles.phone)}>
        <ThreadHeader narrow folder="workstation-a" title="Row projection with a long agent name" status="working" statusLabel="working" freshness="live" sidebarCollapsed onToggleSidebar={() => {}} sidebarSheet={{ open: false, id: 'agents-sheet' }} terminalAvailable panelOpen={false} drawerOpen={false} onTogglePanel={() => {}} onToggleDrawer={() => {}} />
      </div>
      : <ThreadHeader folder="fractal" title="Row projection" panelOpen={false} drawerOpen={false} onTogglePanel={() => {}} onToggleDrawer={() => {}} terminalDisabledReason={terminalDisabledReason} />}
  </main>
}
const meta = { title: 'Fractal UI/Thread header', component: ThreadHeaderStory, parameters: { scenario: { _tag: 'primitive', reason: "Exercises header controls, disabled reasons and responsive geometry without scenario data." }, layout: 'fullscreen' }, args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } } } satisfies Meta<typeof ThreadHeaderStory>
export default meta
type Story = StoryObj<typeof meta>
/** The host knows why the terminal is unavailable: the toggle stays in place and in the tab order, disabled and described; focus shows the reason in the kit tooltip. */
export const TerminalDisabled: Story = { tags: ['critical'], args: { terminalDisabledReason: terminalReason }, play: async ({ canvasElement }) => {
  const header = within(canvasElement).getByTestId('thread-header')
  const toggle = within(header).getByRole('button', { name: 'Toggle terminal drawer' })
  await expect(toggle).toBeVisible()
  await expect(toggle).toHaveAttribute('aria-disabled', 'true')
  await expect(toggle).toHaveAccessibleName('Toggle terminal drawer')
  await expect(toggle).toHaveAccessibleDescription(terminalReason)
  await expect(within(header).getAllByRole('button').map(button => button.getAttribute('aria-label'))).toEqual(['Toggle terminal drawer', 'Toggle right panel'])
  await expect(header.querySelector('[title]')).toBeNull()
  ;(document.activeElement as HTMLElement | null)?.blur()
  await userEvent.tab()
  await expect(toggle).toHaveFocus()
  await waitFor(() => expect(within(document.body).getByRole('tooltip')).toHaveTextContent(`Toggle terminal drawer · ${terminalReason}`))
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(within(document.body).queryByRole('tooltip')).toBeNull())
} }
export const TerminalDisabledLight: Story = { ...TerminalDisabled, args: { ...TerminalDisabled.args, scheme: 'light' } }
/** Without a reason the unavailable terminal leaves no toggle behind. The desktop header keeps its dense 28px controls. */
export const TerminalHidden: Story = { tags: ['critical'], play: async ({ canvasElement }) => {
  const header = within(canvasElement).getByTestId('thread-header')
  await expect(within(header).queryByRole('button', { name: 'Toggle terminal drawer' })).toBeNull()
  const panel = within(header).getByRole('button', { name: 'Toggle right panel' })
  await expect(panel).toBeVisible()
  const { width, height } = panel.getBoundingClientRect()
  await expect({ width, height }).toEqual({ width: 28, height: 28 })
  await expect(within(header).queryByText(terminalReason)).toBeNull()
} }
export const TerminalHiddenLight: Story = { ...TerminalHidden, args: { scheme: 'light' } }
/** The desktop header adapts to its own lane even inside a wide viewport. */
export const ConstrainedLane: Story = { tags: ['critical'], args: { laneWidth: 360 }, play: async ({ canvasElement }) => {
  const header = within(canvasElement).getByTestId('thread-header')
  const lane = header.getBoundingClientRect()
  await expect(lane.height).toBe(32)
  await expect(within(header).getByText('fractal')).not.toBeVisible()
  await expect(within(header).getByRole('heading', { name: 'Row projection' })).toBeVisible()
  const controls = within(header).getAllByRole('button').map(button => button.getBoundingClientRect())
  for (const a of controls) {
    await expect(a.left >= lane.left && a.right <= lane.right && a.top >= lane.top && a.bottom <= lane.bottom).toBe(true)
    for (const b of controls) if (a !== b) await expect(a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top).toBe(false)
  }
} }
export const ConstrainedLaneLight: Story = { ...ConstrainedLane, args: { ...ConstrainedLane.args, scheme: 'light' } }
/**
 * A 390px phone: the header keeps the breadcrumb and status, drops the terminal toggle, and its two controls
 * (Expand sidebar, Agent details) are 44px touch targets that stay inside the header without overflowing it.
 */
export const NarrowPhone: Story = { tags: ['critical'], args: { narrow: true }, play: async ({ canvasElement }) => {
  const header = within(canvasElement).getByTestId('thread-header')
  const frame = within(canvasElement).getByTestId('phone-frame').getBoundingClientRect()
  const buttons = within(header).getAllByRole('button')
  await expect(buttons.map(button => button.getAttribute('aria-label'))).toEqual(['Expand sidebar', 'Agent details'])
  for (const button of buttons) {
    const rect = button.getBoundingClientRect()
    await expect(rect.width).toBeGreaterThanOrEqual(44)
    await expect(rect.height).toBeGreaterThanOrEqual(44)
    await expect(rect.left).toBeGreaterThanOrEqual(frame.left)
    await expect(rect.right).toBeLessThanOrEqual(frame.right)
  }
  await expect(within(header).getByRole('button', { name: 'Expand sidebar' })).toHaveAttribute('aria-expanded', 'false')
  await expect(within(header).getByRole('button', { name: 'Agent details' })).toHaveAttribute('aria-haspopup', 'dialog')
  await expect(header.scrollWidth).toBeLessThanOrEqual(header.clientWidth)
  await expect(within(header).getByRole('navigation', { name: 'Breadcrumb' })).toBeVisible()
} }
export const NarrowPhoneLight: Story = { ...NarrowPhone, args: { narrow: true, scheme: 'light' } }
const styles = stylex.create({
  root: { minHeight: '100vh', boxSizing: 'border-box', backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans },
  phone: { width: '390px', maxWidth: '100%' },
})
