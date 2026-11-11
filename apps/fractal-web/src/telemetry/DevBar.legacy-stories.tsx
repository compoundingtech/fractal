import type { Meta, StoryObj } from '@storybook/react'

import { darkMeterTheme } from '@overeng/meters/canvas'

import { DevBar } from './DevBar.tsx'
import { SocketStateChip } from './transportMeters.tsx'

const meta = {
  title: 'wf/Monitor/Developer Bar',
  component: DevBar,
  parameters: { devBar: false },
  args: { defaultVisible: true },
  // This story owns the real bar; the preview suppresses its singleton to avoid two engines.
  render: (args) => (
    <main>
      <h1>Developer transport bar</h1>
      <p>
        The composable developer shell shows live frame, long-frame and approximate JS-heap meters
        alongside meters for the actual st3 SDK HTTP and collections WebSocket transport. Counters and Meters
        open independent panels. Mod Backquote toggles the selected panel; Mod Shift B toggles visibility.
      </p>
      <p>
        Requests in flight, p50/p95 HTTP response-header latency over the latest 256 responses, HTTP
        errors, shared socket subscriptions and incoming messages per second are each a canvas tile
        with history and an evidence card; a click opens its details. Anything not yet measured reads
        n/a, not zero. WebSocket state is a state element, not a meter. The row also shows one build
        entry: the short source revision and age, including dirty-source semantics, with a dot for
        unread changes. Opening it shows the full source revision, deployment identity and recent
        changes together. The Counters panel retains sorted app measurements,
        including render commits, decode time, retained entries, socket errors, resyncs and retries.
        Meters share one collector session; the Meters panel's strips can be frozen independently.
      </p>
      <DevBar {...args} />
    </main>
  ),
} satisfies Meta<typeof DevBar>
export default meta

type Story = StoryObj<typeof meta>
export const Compact: Story = {}
export const Expanded: Story = {
  play: async ({ canvasElement }) => {
    const button = canvasElement.querySelector<HTMLButtonElement>(
      'button[aria-expanded="false"]:not([aria-label])',
    )
    if (button === null) throw new Error('Counters panel control did not mount')
    button.click()
  },
}
export const AllStates: Story = {
  render: (args) => (
    <main>
      <h1>Developer bar states</h1>
      <p>
        A single live instance keeps measurement ownership unambiguous. Open Counters or Meters,
        use Mod Backquote to toggle a panel and Escape to close it, or freeze a strip in the Meters panel to inspect history.
        Mod Shift B hides and restores the bar; transport values come from actual measurements.
      </p>
      <DevBar {...args} />
    </main>
  ),
}
const socketStates = [
  undefined,
  { state: { _tag: 'Idle' }, sinceMs: 0 },
  { state: { _tag: 'Connecting', attempt: 1 }, sinceMs: 0 },
  { state: { _tag: 'Live' }, sinceMs: 0 },
  { state: { _tag: 'Reconnecting', attempt: 3, issue: 'Socket closed' }, sinceMs: 0 },
  { state: { _tag: 'Rejected', message: 'Token expired' }, sinceMs: 0 },
  { state: { _tag: 'Closed' }, sinceMs: 0 },
] as const
export const SocketStates: Story = {
  render: () => (
    <main style={{ display: 'flex', flexWrap: 'wrap', gap: 12, padding: 12, color: darkMeterTheme.foreground, background: darkMeterTheme.background }}>
      {socketStates.map((observation) => (
        <SocketStateChip key={observation?.state._tag ?? 'Unobserved'} observation={observation} theme={darkMeterTheme} />
      ))}
    </main>
  ),
}
