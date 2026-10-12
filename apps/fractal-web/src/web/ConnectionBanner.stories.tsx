import * as React from 'react'
import * as Aria from 'react-aria-components'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, userEvent, within } from 'storybook/test'
import { ConnectionBanner } from './ConnectionBanner.tsx'
import { composerConnectionProps } from './composerConnection.ts'

const ConnectionSurface = ({ initial = 'live' }: { readonly initial?: 'live' | 'offline' | 'reconnecting' }) => {
  const [state, setState] = React.useState(initial)
  const { connectionNotice } = composerConnectionProps({ network: { _tag: state === 'offline' ? 'Offline' : 'Online' }, observation: state === 'reconnecting' ? { status: { _tag: 'Stale', reason: { _tag: 'Reconnecting', attempt: 2, nextAt: 6000, issue: 'transport lost' }, lastLiveAt: 0 }, observedAt: 1000 } : { status: { _tag: 'Live', since: 0 }, observedAt: 0 }, now: 5000, reconnect: () => setState('live') })
  return <section style={{ maxWidth: 680 }}>
    <Aria.Button onPress={() => setState('offline')}>Go offline</Aria.Button>
    <Aria.Button onPress={() => setState('reconnecting')}>Lose connection</Aria.Button>
    <ConnectionBanner notice={connectionNotice} retained />
    <article data-retained-content=""><h2>Observed result</h2><p>The review completed successfully.</p></article>
  </section>
}
const assertRetained = (node: HTMLElement): void => { if (node.textContent !== 'Observed resultThe review completed successfully.') throw new Error('Connection loss must preserve observed content') }
const meta = { title: 'wf/Connection/Retained content', component: ConnectionSurface, args: { initial: 'live' }, parameters: { docs: { description: { component: 'Offline and reconnect notices are independent shell chrome. Retained data stays mounted and is explicitly labelled last observed, never silently replaced by a loading or error row.' } } } } satisfies Meta<typeof ConnectionSurface>
export default meta
type Story = StoryObj<typeof meta>
export const Reconnect: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement), retained = canvasElement.querySelector<HTMLElement>('[data-retained-content]')!
  await expect(canvas.queryByRole('complementary', { name: 'Connection status' })).toBeNull()
  await userEvent.click(canvas.getByRole('button', { name: 'Go offline' }))
  await expect(canvas.getByRole('status')).toHaveTextContent('Last observed content stays visible and may be out of date.')
  assertRetained(retained)
  const saved = retained.innerHTML
  retained.innerHTML = ''
  try { await expect(() => assertRetained(retained)).toThrow('Connection loss must preserve observed content') }
  finally { retained.innerHTML = saved }
  await userEvent.click(canvas.getByRole('button', { name: 'Lose connection' }))
  await expect(canvas.getByRole('status')).toHaveTextContent('Connection lost · reconnecting')
  assertRetained(retained)
  await userEvent.click(canvas.getByRole('button', { name: 'Reconnect' }))
  await expect(canvas.queryByRole('complementary', { name: 'Connection status' })).toBeNull()
  await expect(canvasElement.querySelector('[data-retained-content]')).toBe(retained)
  assertRetained(retained)
} }
export const Offline: Story = { args: { initial: 'offline' } }
export const AllStates: Story = { render: () => <>{(['live', 'offline', 'reconnecting'] as const).map(initial => <section key={initial}><h2>{initial}</h2><ConnectionSurface initial={initial} /></section>)}</> }
