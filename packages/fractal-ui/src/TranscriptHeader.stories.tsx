import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button } from 'react-aria-components'
import { Transcript } from './assistant-ui/composition/Transcript.tsx'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime.tsx'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const now = Date.parse('2026-01-15T12:00:00Z')
const variants = [
  { width: 440, scheme: 'light' }, { width: 440, scheme: 'dark' },
  { width: 1440, scheme: 'light' }, { width: 1440, scheme: 'dark' },
] as const
const transcriptProps = { title: 'Row projection', turns: [], sync: { _tag: 'Live', since: now }, now, observedAt: now } as const
function HeaderVariant({ width, scheme }: { width: number; scheme: Scheme }) {
  const [showHeader, setShowHeader] = React.useState(true)
  return <section aria-label={`${width}px ${scheme}`} data-scheme={scheme} {...stylex.props(styles.variant(width), ...baselineTheme, scheme === 'light' && lightTheme)}>
    <h2>{width}px · {scheme}</h2>
    {/* Every region label carries its variant: axe requires each landmark on the page to be distinguishable. */}
    <section aria-label={`Host-owned header, ${width}px ${scheme}`} {...stylex.props(styles.frame)}>
      {/* Like the live workspace, the host header is a labelled group inside the page, not a top-level banner. */}
      <div role="group" aria-label="Workspace header">Row projection</div>
      <EmbraceRuntimeProvider options={{ messages: [], isRunning: false, onNew: async () => {} }}>
        <Transcript {...transcriptProps} showHeader={false} />
      </EmbraceRuntimeProvider>
    </section>
    <section aria-label={`Standalone control, ${width}px ${scheme}`} {...stylex.props(styles.frame)}>
      <EmbraceRuntimeProvider options={{ messages: [], isRunning: false, onNew: async () => {} }}>
        <Transcript {...transcriptProps} />
      </EmbraceRuntimeProvider>
    </section>
    <section aria-label={`Interactive header, ${width}px ${scheme}`} {...stylex.props(styles.frame)}>
      <Button onPress={() => setShowHeader(value => !value)}>{showHeader ? 'Hide kit header' : 'Show kit header'}</Button>
      <EmbraceRuntimeProvider options={{ messages: [], isRunning: false, onNew: async () => {} }}>
        <Transcript {...transcriptProps} showHeader={showHeader} />
      </EmbraceRuntimeProvider>
    </section>
  </section>
}
const HeaderSurface = () => <main {...stylex.props(styles.matrix)}>{variants.map(variant => <HeaderVariant key={`${variant.width}-${variant.scheme}`} {...variant} />)}</main>
const meta = { title: 'Fractal UI/Transcript header ownership', component: HeaderSurface, parameters: { layout: 'fullscreen' } } satisfies Meta<typeof HeaderSurface>
export default meta
type Story = StoryObj<typeof meta>
export const AllStates: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  for (const { width, scheme } of variants) {
    const variant = within(canvas.getByRole('region', { name: `${width}px ${scheme}` }))
    const hosted = within(variant.getByRole('region', { name: `Host-owned header, ${width}px ${scheme}` }))
    await expect(hosted.queryByTestId('transcript-header')).toBeNull()
    await expect(hosted.getByRole('group', { name: 'Workspace header' })).toBeVisible()
    await expect(await hosted.findByTestId('transcript-scroll')).toBeVisible()
    // Failing control: omitting showHeader must retain the default header.
    const standalone = within(variant.getByRole('region', { name: `Standalone control, ${width}px ${scheme}` }))
    await expect(await standalone.findByTestId('transcript-header')).toBeVisible()
    const interactive = within(variant.getByRole('region', { name: `Interactive header, ${width}px ${scheme}` }))
    await expect(await interactive.findByTestId('transcript-header')).toBeVisible()
    await userEvent.click(interactive.getByRole('button', { name: 'Hide kit header' }))
    await waitFor(() => expect(interactive.queryByTestId('transcript-header')).toBeNull())
    await expect(await interactive.findByTestId('transcript-scroll')).toBeVisible()
    await userEvent.click(await interactive.findByRole('button', { name: 'Show kit header' }))
    await expect(await interactive.findByTestId('transcript-header')).toBeVisible()
    await expect(await interactive.findByTestId('transcript-scroll')).toBeVisible()
  }
  canvasElement.dataset.proof = 'passed'
} }
const styles = stylex.create({
  matrix: { display: 'flex', flexDirection: 'column', gap: s.xl },
  variant: (width: number) => ({ width, flexShrink: 0, padding: s.xl, boxSizing: 'border-box', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans }),
  frame: { height: g.transcriptFrameHeight, display: 'flex', flexDirection: 'column', minHeight: 0 },
})
