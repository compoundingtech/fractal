import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { ThreadHeader } from './assistant-ui/composition/Shell'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { colorVars as c, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const terminalReason = 'The terminal opens once this agent is running on a host.'
function ThreadHeaderStory({ scheme = 'dark', terminalDisabledReason, laneWidth }: { scheme?: Scheme; terminalDisabledReason?: string; laneWidth?: number }) {
  return <main data-scheme={scheme} style={laneWidth === undefined ? undefined : { width: laneWidth }} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <ThreadHeader folder="fractal" title="Row projection" panelOpen={false} drawerOpen={false} onTogglePanel={() => {}} onToggleDrawer={() => {}} terminalDisabledReason={terminalDisabledReason} />
  </main>
}
const meta = { title: 'Fractal UI/Thread header', component: ThreadHeaderStory, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } } } satisfies Meta<typeof ThreadHeaderStory>
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
/** Without a reason the unavailable terminal leaves no toggle behind. */
export const TerminalHidden: Story = { tags: ['critical'], play: async ({ canvasElement }) => {
  const header = within(canvasElement).getByTestId('thread-header')
  await expect(within(header).queryByRole('button', { name: 'Toggle terminal drawer' })).toBeNull()
  await expect(within(header).getByRole('button', { name: 'Toggle right panel' })).toBeVisible()
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
const styles = stylex.create({
  root: { minHeight: '100vh', boxSizing: 'border-box', backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans },
})
