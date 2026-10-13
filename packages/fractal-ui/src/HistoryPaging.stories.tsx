import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { ConversationPane } from '../../../apps/fractal-web/src/web/ConversationPane.tsx'
import { DataSourceProvider } from '../../../apps/fractal-web/src/data/react.tsx'
import { createHistoryFixture, historyFixtureRef, type HistoryFixture } from '../../../apps/fractal-web/src/data/historyFixture.ts'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, spaceVars as s } from './assistant-ui/composition-tokens.stylex'

const fixtures = new WeakMap<HTMLElement, HistoryFixture>()
const HistoryPaging = ({ scheme = 'dark', failure = 'none' }: { readonly scheme?: 'light' | 'dark'; readonly failure?: 'none' | 'retry' }) => {
  const fixture = React.useMemo(() => createHistoryFixture(), [])
  React.useLayoutEffect(() => () => fixture.registry.dispose(), [fixture])
  const source = React.useMemo(() => failure === 'retry' ? { ...fixture.source, loadEarlierConversation: async (ref: string) => {
    const feed = fixture.registry.get(fixture.source.conversation(ref))
    if (feed._tag === 'Observed' && feed.value.historyLoad?._tag === 'Failed') return
    await fixture.source.loadEarlierConversation?.(ref)
  } } : fixture.source, [failure, fixture])
  return <main ref={node => { if (node !== null) fixtures.set(node, fixture) }} data-history-paging-story="" data-scheme={scheme} {...stylex.props(...baselineTheme, scheme === 'light' && lightTheme)} style={{ display: 'flex', flexDirection: 'column', height: '100vh', width: '100%', background: surface.canvas, color: ink.fg }}>
    <div style={{ display: 'flex', gap: s.md, padding: s.sm }}><button onClick={() => fixture.settle(false)}>Fail pending page</button><button onClick={() => fixture.settle(true)}>Deliver pending page</button></div>
    <DataSourceProvider source={source} registry={fixture.registry}><ConversationPane agentRef={historyFixtureRef} agentName="Row review" onOpenTool={() => {}} /></DataSourceProvider>
  </main>
}
const meta = { title: 'Fractal UI/Native history paging', component: HistoryPaging, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark', failure: 'none' }, argTypes: { scheme: { control: 'radio', options: ['light', 'dark'] }, failure: { control: 'select', options: ['none', 'retry'] } } } satisfies Meta<typeof HistoryPaging>
export default meta
type Story = StoryObj<typeof meta>
/** Manual state tour uses the same source and production pane as the recovery play. */
export const AllStates: Story = {}
export const Recovery: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const root = canvasElement.querySelector<HTMLElement>('[data-history-paging-story]')!
  const fixture = fixtures.get(root)!
  const feed = () => fixture.registry.get(fixture.source.conversation(historyFixtureRef))
  await waitFor(() => expect(feed()).toMatchObject({ _tag: 'Observed', value: { historyLoad: { _tag: 'Loading' } } }))
  await userEvent.click(canvas.getByRole('button', { name: 'Fail pending page' }))
  await waitFor(() => expect(feed()).toMatchObject({ _tag: 'Observed', value: { historyLoad: { _tag: 'Failed' } } }))
  await waitFor(() => expect(canvas.getByRole('link', { name: 'Try again' })).toBeVisible())
  await userEvent.click(canvas.getByRole('link', { name: 'Try again' }))
  await waitFor(() => expect(feed()).toMatchObject({ _tag: 'Observed', value: { historyLoad: { _tag: 'Loading' } } }))
  await userEvent.click(canvas.getByRole('button', { name: 'Deliver pending page' }))
  await waitFor(() => expect(feed()).toMatchObject({ _tag: 'Observed', value: { hasOlder: false } }))
  await waitFor(() => expect(canvas.getByText('The earlier review identified the stable row keys.')).toBeVisible())
  await expect(canvas.getByText('Selection is retained in the newest result.')).toBeVisible()
  await expect(canvas.queryByRole('link', { name: 'Try again' })).toBeNull()
} }
