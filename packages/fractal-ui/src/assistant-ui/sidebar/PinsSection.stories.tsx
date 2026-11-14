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
    <div data-state="readonly"><PinsSection title="Pinned · read-only" items={ordered} editable={false} onOpen={setOpened} onUnpin={() => undefined} /></div>
    <div data-state="failed"><PinsSection title="Pinned · unavailable" items={ordered.slice(0, 1)} editable={false} onOpen={setOpened} onUnpin={() => undefined} detail="Pins could not be saved" onRetry={() => setRetries(count => count + 1)} /></div>
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
export const scenarioExemptions: readonly string[] = ['AllStates:contract-fixture', 'ManyPins:contract-fixture']

export const AllStates: StoryObj<typeof meta> = { tags: ['critical', 'both-widths'], name: 'All states · ordered, read-only, failed, empty', play: async ({ canvasElement }) => {
  const editable = state(canvasElement, 'editable')
  await expect(editable.getByRole('region', { name: 'Pinned' })).toBeInTheDocument()
  await expect(state(canvasElement, 'readonly').getByRole('region', { name: 'Pinned · read-only' })).toBeInTheDocument()
  await expect(state(canvasElement, 'failed').getByRole('region', { name: 'Pinned · unavailable' })).toBeInTheDocument()
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

const many = Array.from({ length: 24 }, (_, index) => ({ id: `agent/pin-${index + 1}`, label: `Pinned item ${index + 1}` }))
export const ManyPins: StoryObj<typeof meta> = {
  tags: ['critical', 'both-widths'],
  render: () => <main data-testid="pins-roster" {...stylex.props(styles.many)}>
    <PinsSection items={many} editable onOpen={() => undefined} onUnpin={() => undefined} />
    <div data-testid="agent-tree" {...stylex.props(styles.tree)}>Agent tree keeps its remaining space</div>
  </main>,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement), roster = canvas.getByTestId('pins-roster')
    const section = canvas.getByRole('region', { name: 'Pinned' }), list = canvas.getByRole('list')
    await expect(section.getBoundingClientRect().height).toBeLessThanOrEqual(roster.clientHeight * 0.4 + 1)
    await expect(canvas.getByTestId('agent-tree').getBoundingClientRect().height).toBeGreaterThanOrEqual(roster.clientHeight * 0.6 - 1)
    await expect(list.scrollHeight).toBeGreaterThan(list.clientHeight)
    canvas.getByRole('button', { name: 'Unpin Pinned item 23' }).focus()
    await userEvent.tab()
    const last = canvas.getByRole('button', { name: 'Pinned item 24' })
    await expect(last).toHaveFocus()
    await expect(list.scrollTop).toBeGreaterThan(0)
    await expect(last.getBoundingClientRect().top).toBeGreaterThanOrEqual(list.getBoundingClientRect().top)
    await expect(last.getBoundingClientRect().bottom).toBeLessThanOrEqual(list.getBoundingClientRect().bottom + 1)
  },
}

const styles = stylex.create({
  canvas: { display: 'flex', flexDirection: 'column', gap: s.lg, width: g.sidebarDefault, maxWidth: '100%', boxSizing: 'border-box', padding: s.sm, minHeight: '100vh', backgroundColor: surface.canvas, color: ink.fg },
  many: { display: 'flex', flexDirection: 'column', height: 'min(600px, 80vh)', width: g.sidebarDefault, maxWidth: '100%', overflow: 'hidden', backgroundColor: surface.canvas, color: ink.fg },
  tree: { flex: '1 1 0', minHeight: 0, overflowY: 'auto' },
})
