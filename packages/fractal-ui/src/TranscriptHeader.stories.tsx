import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { geometryVars as g, surfaceVars as surface, spaceVars as s, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { Transcript } from './assistant-ui/composition/Transcript.tsx'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime.tsx'
import { PaneHeader } from './assistant-ui/composition/Pane'

const now = Date.parse('2026-01-15T12:00:00Z')
const styles = stylex.create({
  root: { width: g.polishWide, maxWidth: '100%', backgroundColor: surface.canvas, color: ink.fg, padding: s.md },
  narrow: { width: g.polishNarrow },
  hosted: { height: 300, padding: 0, display: 'flex', flexDirection: 'column' },
  title: { margin: 0, fontSize: t.metaSize, fontWeight: t.weightMedium },
})
const HeaderSurface = ({ scheme = 'dark', narrow = false }: { scheme?: Scheme; narrow?: boolean }) => <main {...stylex.props(styles.root, narrow && styles.narrow, ...baselineTheme, scheme === 'light' && lightTheme)}>{[false, true].map(showHeader => <section key={String(showHeader)} aria-label={showHeader ? 'Standalone control' : 'Host-owned header'} style={{ height: 300 }}>
  {!showHeader && <header role="group" aria-label="Workspace header">Row projection</header>}
  <EmbraceRuntimeProvider options={{ messages: [], isRunning: false, onNew: async () => {} }}>
    <Transcript title="Row projection" turns={[]} showHeader={showHeader} sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} />
  </EmbraceRuntimeProvider>
</section>)}</main>
const meta = { title: 'Fractal UI/Transcript header ownership', component: HeaderSurface, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark', narrow: false } } satisfies Meta<typeof HeaderSurface>
export default meta
type Story = StoryObj<typeof meta>
export const Dark1440: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const hosted = within(canvas.getByRole('region', { name: 'Host-owned header' }))
  await expect(hosted.queryByTestId('transcript-header')).toBeNull()
  await expect(hosted.getByRole('group', { name: 'Workspace header' })).toBeVisible()
  await expect(hosted.getByTestId('transcript-scroll')).toBeVisible()
  // Failing control: standalone presentation retains the band by default.
  await expect(within(canvas.getByRole('region', { name: 'Standalone control' })).getByTestId('transcript-header')).toBeVisible()
  canvasElement.dataset.proof = 'passed'
} }
export const Light1440: Story = { ...Dark1440, args: { scheme: 'light' } }
export const Dark440: Story = { ...Dark1440, args: { narrow: true } }
export const Light440: Story = { ...Dark1440, args: { narrow: true, scheme: 'light' } }
export const AllStates: Story = { ...Dark1440 }

function HostedStatusSurface({ scheme = 'dark' }: { scheme?: Scheme }) {
  const [host, setHost] = React.useState<HTMLDivElement | null>(null)
  const [active, setActive] = React.useState(true)
  return <main {...stylex.props(styles.root, styles.hosted, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <PaneHeader actions={<div ref={setHost} />}><h2 {...stylex.props(styles.title)}>Row projection</h2></PaneHeader>
    <button type="button" onClick={() => setActive(value => !value)}>{active ? 'Park transcript' : 'Activate transcript'}</button>
    <EmbraceRuntimeProvider options={{ messages: [], isRunning: false, onNew: async () => {} }}>
      <Transcript title="Row projection" turns={[]} showHeader={false} headerActionsHost={host} active={active}
        sync={{ _tag: 'Progress', stage: 'reading', done: 1, total: 2, elapsedMs: 100, stageSince: now, reportedAt: now }}
        now={now} observedAt={now} loadingState={<p>History is loading.</p>} />
    </EmbraceRuntimeProvider>
  </main>
}

export const HostedStatus: Story = {
  render: args => <HostedStatusSurface scheme={args.scheme} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getAllByRole('heading', { name: 'Row projection' })).toHaveLength(1)
    await expect(canvas.queryByTestId('transcript-header')).toBeNull()
    const header = canvas.getByTestId('pane-header')
    const progress = within(header).getByRole('progressbar', { name: 'Thread synchronization' })
    await expect(progress).toBeVisible()
    await expect(progress).toHaveAttribute('aria-valuenow', '1')
    await expect(header.getBoundingClientRect().height).toBe(32)
    await expect(progress.getBoundingClientRect().width).toBeGreaterThan(0)
    await userEvent.click(canvas.getByRole('button', { name: 'Park transcript' }))
    await expect(within(header).queryByRole('progressbar')).toBeNull()
    await userEvent.click(canvas.getByRole('button', { name: 'Activate transcript' }))
    await expect(within(header).getByRole('progressbar', { name: 'Thread synchronization' })).toBeVisible()
  },
}
export const HostedStatusLight: Story = { ...HostedStatus, args: { scheme: 'light' } }
