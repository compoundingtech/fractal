import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'
import { Transcript } from './assistant-ui/composition/Transcript.tsx'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime.tsx'

const now = Date.parse('2026-01-15T12:00:00Z')
const HeaderSurface = () => <main>{[false, true].map(showHeader => <section key={String(showHeader)} aria-label={showHeader ? 'Standalone control' : 'Host-owned header'} style={{ height: 300 }}>
  {!showHeader && <header role="banner" aria-label="Workspace header">Row projection</header>}
  <EmbraceRuntimeProvider options={{ messages: [], isRunning: false, onNew: async () => {} }}>
    <Transcript title="Row projection" turns={[]} showHeader={showHeader} sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} />
  </EmbraceRuntimeProvider>
</section>)}</main>
const meta = { title: 'Fractal UI/Transcript header ownership', component: HeaderSurface } satisfies Meta<typeof HeaderSurface>
export default meta
type Story = StoryObj<typeof meta>
export const AllStates: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const hosted = within(canvas.getByRole('region', { name: 'Host-owned header' }))
  await expect(hosted.queryByTestId('transcript-header')).toBeNull()
  await expect(hosted.getByRole('banner', { name: 'Workspace header' })).toBeVisible()
  await expect(hosted.getByTestId('transcript-scroll')).toBeVisible()
  // Failing control: standalone presentation retains the band by default.
  await expect(within(canvas.getByRole('region', { name: 'Standalone control' })).getByTestId('transcript-header')).toBeVisible()
  canvasElement.dataset.proof = 'passed'
} }
