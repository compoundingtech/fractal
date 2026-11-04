/** Pins section: server order, Open and separate Unpin, read-only, failure retry and the empty state. Synthetic hosts only. */
import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import * as stylex from '@stylexjs/stylex'
import { expect, userEvent, within } from 'storybook/test'
import { PinsSection } from './PinsSection'
import { geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink } from '../composition-tokens.stylex'

const ordered = [{ id: 'mission/release', label: 'Release train' }, { id: 'agent/reviewer', label: 'Reviewer' }, { id: 'agent/a-very-long-synthetic-agent-name-that-truncates', label: 'A very long synthetic agent name that truncates' }] as const

function AllStatesHarness() {
  const [items, setItems] = React.useState<readonly { id: string; label: string }[]>(ordered)
  const [opened, setOpened] = React.useState('')
  const [retries, setRetries] = React.useState(0)
  return <main {...stylex.props(styles.canvas)}>
    <div data-state="editable"><PinsSection items={items} editable onOpen={setOpened} onUnpin={id => setItems(current => current.filter(item => item.id !== id))} /></div>
    <div data-state="readonly"><PinsSection items={ordered} editable={false} onOpen={setOpened} onUnpin={() => undefined} /></div>
    <div data-state="failed"><PinsSection items={ordered.slice(0, 1)} editable={false} onOpen={setOpened} onUnpin={() => undefined} detail="Pins could not be saved" onRetry={() => setRetries(count => count + 1)} /></div>
    <div data-state="empty"><PinsSection items={[]} editable onOpen={setOpened} onUnpin={() => undefined} /></div>
    <output data-testid="pins-log">{`${opened} retries:${retries}`}</output>
  </main>
}

const state = (root: HTMLElement, name: string) => within(root.querySelector<HTMLElement>(`[data-state="${name}"]`)!)
const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Sidebar/Pins Section',
  component: AllStatesHarness,
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Host-owned synthetic Pins exercise server order, open, unpin, read-only, retry and empty states independently of catalog data.' }, layout: 'fullscreen' },
} satisfies Meta<typeof AllStatesHarness>
export default meta
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['AllStates:contract-fixture']

export const AllStates: StoryObj<typeof meta> = { tags: ['critical'], name: 'All states · ordered, read-only, failed, empty', play: async ({ canvasElement }) => {
  const editable = state(canvasElement, 'editable')
  await expect(editable.getByRole('region', { name: 'Pinned' })).toBeInTheDocument()
  await expect(editable.getAllByRole('listitem').map(item => item.dataset.pin)).toEqual(ordered.map(item => item.id))
  await userEvent.click(editable.getByRole('button', { name: 'Reviewer' }))
  await expect(within(canvasElement).getByTestId('pins-log')).toHaveTextContent('agent/reviewer')
  await userEvent.click(editable.getByRole('button', { name: 'Unpin Reviewer' }))
  await expect(editable.getAllByRole('listitem').map(item => item.dataset.pin)).toEqual([ordered[0].id, ordered[2].id])
  const label = editable.getByRole('button', { name: ordered[2].label }).firstElementChild!
  await expect(label.scrollWidth).toBeGreaterThan(label.clientWidth)
  const readonly = state(canvasElement, 'readonly')
  for (const item of ordered) await expect(readonly.getByRole('button', { name: `Unpin ${item.label}` })).toBeDisabled()
  await userEvent.click(readonly.getByRole('button', { name: 'Release train' }))
  await expect(within(canvasElement).getByTestId('pins-log')).toHaveTextContent('mission/release')
  const failed = state(canvasElement, 'failed')
  await expect(failed.getByRole('status')).toHaveTextContent('Pins could not be saved')
  await userEvent.click(failed.getByRole('button', { name: 'Retry: Pins could not be saved' }))
  await expect(within(canvasElement).getByTestId('pins-log')).toHaveTextContent('retries:1')
  await expect(canvasElement.querySelector('[data-state="empty"]')).toBeEmptyDOMElement()
} }

const styles = stylex.create({
  canvas: { display: 'flex', flexDirection: 'column', gap: s.lg, width: g.sidebarDefault, maxWidth: '100%', boxSizing: 'border-box', padding: s.sm, minHeight: '100vh', backgroundColor: surface.canvas, color: ink.fg },
})
