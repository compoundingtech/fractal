/** This browser's outbox recovery list: the production component over a deterministic in-memory port, one story per attempt state. */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { Tab, TabList, TabPanel, Tabs } from 'react-aria-components'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui'
import { accentVars as accent, geometryVars, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { OutboxRecoveryCopy, OutboxRecoveryItem, OutboxRecoveryPort } from '../data/source.ts'
import { OutboxRecovery, RECOVERY_TITLE, RECOVERY_UNAVAILABLE } from './OutboxRecovery.tsx'

type State = 'unknown' | 'not-applied' | 'pending' | 'corrupt' | 'identity-mismatch' | 'unavailable'
type Args = { readonly scheme: 'dark' | 'light'; readonly state: State }
const states: readonly State[] = ['unknown', 'not-applied', 'pending', 'corrupt', 'identity-mismatch', 'unavailable']
const stateLabel: Readonly<Record<State, string>> = {
  unknown: 'Unconfirmed', 'not-applied': 'Not applied', pending: 'Pending', corrupt: 'Damaged', 'identity-mismatch': 'Other identity', unavailable: 'Storage unavailable',
}

// A long unbroken run must wrap or truncate inside the 440px lane, never widen it.
const longReference = 'https://example.test/releases/2026-11/notes/0123456789abcdef0123456789abcdef0123456789abcdef'
const fixtures: Readonly<Record<Exclude<State, 'unavailable'>, readonly OutboxRecoveryItem[]>> = {
  unknown: [
    { id: 'outbox/attempt/unknown-reply', kind: 'unknown', subject: 'message', status: 'delivery unconfirmed', actions: ['copy', 'discard'], text: `Ship the release notes from ${longReference}` },
    { id: 'outbox/attempt/unknown-start', kind: 'unknown', subject: 'thread', status: 'delivery unconfirmed', actions: ['copy', 'discard'], text: 'Release checklist for 4.2' },
  ],
  'not-applied': [
    { id: 'outbox/attempt/not-applied', kind: 'not-applied', subject: 'message', status: 'Fractal refused it: the conversation was closed.', actions: ['retry', 'discard'], text: 'Please rerun the migration check.' },
  ],
  pending: [
    { id: 'outbox/attempt/pending-start', kind: 'pending', subject: 'thread', status: 'Saved before sending; not yet submitted.', actions: ['send', 'discard'], text: 'Weekly release review' },
    { id: 'outbox/attempt/pending-reply', kind: 'pending', subject: 'message', status: 'Saved before sending; not yet submitted.', actions: ['send', 'discard'], text: 'Approved; go ahead with def.' },
  ],
  corrupt: [
    { id: 'outbox/attempt/corrupt', kind: 'corrupt', subject: 'message', status: `storage damaged; delivery unknown (${longReference})`, actions: ['discard'] },
  ],
  'identity-mismatch': [
    { id: 'outbox/attempt/foreign', kind: 'identity-mismatch', subject: 'message', status: 'Saved for issuer host/build-2, not this connection.', actions: ['discard'] },
  ],
}
const copied: Omit<OutboxRecoveryCopy, 'copyKey'> = { text: `Ship the release notes from ${longReference}`, images: [{ name: 'release-diagram-with-a-long-descriptive-file-name.png', mediaType: 'image/png', bytes: 48_213 }] }
const refusal = 'The recipient could not be verified.'

/**
 * An in-memory port: every command is recorded in the visible receipt log and changes the list exactly as the outbox
 * would. Each copy gets its own key; the first send of a copy is refused (nothing saved) and later sends record it.
 */
const createFixturePort = (state: State, record: (receipt: string) => void): OutboxRecoveryPort => {
  let items = state === 'unavailable' ? [] : fixtures[state]
  const listeners = new Set<() => void>()
  let copies = 0
  const refused = new Set<string>()
  const remove = (id: string) => {
    items = items.filter(item => item.id !== id)
    for (const listener of listeners) listener()
  }
  return {
    list: async () => { if (state === 'unavailable') throw new Error('IndexedDB unavailable'); return items },
    discard: async id => { record(`discard ${id}`); remove(id) },
    subscribe: listener => { listeners.add(listener); return () => { listeners.delete(listener) } },
    retry: async id => { record(`retry ${id}`); remove(id) },
    send: async id => { record(`send ${id}`); remove(id) },
    copy: async id => { record(`copy ${id}`); return { ...copied, copyKey: `outbox/copy/${++copies}` } },
    sendCopy: async (id, copyKey, text) => {
      record(`sendCopy ${id} as ${copyKey}: ${text}`)
      if (refused.has(copyKey)) return { _tag: 'Recorded' }
      refused.add(copyKey)
      return { _tag: 'Refused', detail: refusal }
    },
  }
}

function RecoveryStory({ scheme, state }: Args) {
  const [receipts, record] = React.useReducer((previous: readonly string[], receipt: string) => [...previous, receipt], [])
  const [port] = React.useState(() => createFixturePort(state, record))
  return <div data-scheme={scheme} data-outbox-story={state} {...stylex.props(styles.canvas, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
    <OutboxRecovery port={port} />
    <div role="log" aria-label="Fixture outbox commands" {...stylex.props(styles.log)}>{receipts.map((receipt, index) => <p key={index} {...stylex.props(styles.receipt)}>{receipt}</p>)}</div>
  </div>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/Outbox recovery',
  component: RecoveryStory,
  args: { scheme: 'dark', state: 'unknown' },
  argTypes: {
    scheme: { options: ['dark', 'light'], control: 'inline-radio' },
    state: { options: states, control: 'select' },
  },
  parameters: {
    layout: 'fullscreen',
    scenario: { _tag: 'contract-fixture', reason: 'Browser outbox attempts in each recovery state over an in-memory port with recorded commands; no world slice models this device’s outbox and nothing is sent.' },
    docs: { description: { component: 'Attempts this browser holds but cannot send by itself. An unconfirmed original may have arrived: it is only copied into a separate new attempt, never retried. Not-applied attempts retry as a fresh attempt; pending ones revalidate before sending; damaged or foreign entries can only be discarded, and Discard always asks first. Unreadable storage is a notice without actions.' } },
  },
  render: args => <main><RecoveryStory key={`${args.state}:${args.scheme}`} {...args} /></main>,
} satisfies Meta<Args>
export default meta
type Story = StoryObj<Args>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStates:contract-fixture',
  'Corrupt:contract-fixture',
  'IdentityMismatch:contract-fixture',
  'NotApplied:contract-fixture',
  'Pending:contract-fixture',
  'Unavailable:contract-fixture',
  'Unknown:contract-fixture',
]

const region = (canvasElement: HTMLElement) => waitFor(() => within(canvasElement).getByRole('region', { name: RECOVERY_TITLE }))
const row = (canvasElement: HTMLElement, id: string) => {
  const found = canvasElement.querySelector<HTMLElement>(`[data-outbox-item="${id}"]`)
  if (found === null) throw new Error(`no outbox row ${id}`)
  return found
}
const rowActions = (element: HTMLElement) => within(element).queryAllByRole('button').map(button => button.textContent)
const receipts = (canvasElement: HTMLElement) => [...canvasElement.querySelectorAll('[role="log"] p')].map(line => line.textContent)

/** Nothing scrolls sideways: the page, the list, and every row and control stay inside the lane. */
const expectFits = async (canvasElement: HTMLElement, scope: HTMLElement) => {
  await expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(document.documentElement.clientWidth)
  await expect(scope.scrollWidth).toBeLessThanOrEqual(scope.clientWidth + 1)
  const bounds = scope.getBoundingClientRect()
  for (const element of scope.querySelectorAll<HTMLElement>('[data-outbox-item], button, textarea')) {
    const box = element.getBoundingClientRect()
    await expect(box.left).toBeGreaterThanOrEqual(bounds.left - 1)
    await expect(box.right).toBeLessThanOrEqual(bounds.right + 1)
  }
  await expect(canvasElement.scrollWidth).toBeLessThanOrEqual(canvasElement.clientWidth + 1)
}

/** Discard asks first; Keep closes the confirmation without touching the outbox. */
const keepThroughConfirmation = async (canvasElement: HTMLElement, id: string, consequence: string) => {
  await userEvent.click(within(row(canvasElement, id)).getByRole('button', { name: /^Discard/ }))
  const dialog = await within(canvasElement).findByRole('alertdialog', { name: 'Discard from this device?' })
  await expect(dialog).toHaveAccessibleDescription(consequence)
  await expect(within(dialog).getByRole('button', { name: 'Keep' })).toHaveFocus()
  await expectFits(canvasElement, dialog)
  await userEvent.click(within(dialog).getByRole('button', { name: 'Keep' }))
  await waitFor(() => expect(within(canvasElement).queryByRole('alertdialog')).toBeNull())
  await expect(receipts(canvasElement)).toEqual([])
}

const unknownPlay: Story['play'] = async ({ canvasElement }) => {
  const list = await region(canvasElement)
  const reply = row(canvasElement, 'outbox/attempt/unknown-reply')
  const start = row(canvasElement, 'outbox/attempt/unknown-start')
  await expect(within(reply).getByText('Fractal has not confirmed this message. Do not send it again.')).toBeVisible()
  await expect(within(start).getByText('Fractal has not confirmed this thread. Do not send it again.')).toBeVisible()
  // The unconfirmed headline is the row's live status, so a change into this state is announced.
  for (const element of [reply, start]) await expect(within(element).getByRole('status')).toHaveTextContent(/^Fractal has not confirmed this (message|thread)\. Do not send it again\.$/)
  // No retry, re-send or delivery status for an unconfirmed original.
  for (const element of [reply, start]) await expect(rowActions(element)).toEqual(['Copy', 'Discard'])
  await expect(list.textContent).not.toMatch(/Retry|Revalidate|delivery unconfirmed/)
  await expectFits(canvasElement, list)
  await keepThroughConfirmation(canvasElement, 'outbox/attempt/unknown-reply', 'This removes the copy on this device only. It does not cancel the original, which may already have arrived.')

  await userEvent.click(within(reply).getByRole('button', { name: /^Copy/ }))
  const composer = await within(canvasElement).findByRole('dialog', { name: 'Send a copy as a new message' })
  await expect(composer).toHaveAccessibleDescription('The original may have arrived. Fractal has not confirmed it, so this copy is sent as a separate new message and both may appear.')
  const field = within(composer).getByRole('textbox', { name: 'Copy of the message' })
  await expect(field).toHaveValue(copied.text)
  await expect(within(composer).getByRole('list', { name: 'Sends with 1 image from the original' })).toHaveTextContent(copied.images[0]!.name)
  await expectFits(canvasElement, composer)
  await userEvent.type(field, ' (copy)')
  // Refused: nothing was saved, so the composer stays with the edited text and may send again under the same key.
  await userEvent.click(within(composer).getByRole('button', { name: 'Send copy' }))
  await expect(await within(composer).findByRole('alert')).toHaveTextContent(`Fractal refused this copy and nothing new was saved: ${refusal} Your text is still here.`)
  await expect(field).toHaveValue(`${copied.text} (copy)`)
  await userEvent.click(within(composer).getByRole('button', { name: 'Send copy' }))
  await waitFor(() => expect(within(canvasElement).queryByRole('dialog')).toBeNull())
  const sent = `sendCopy outbox/attempt/unknown-reply as outbox/copy/1: ${copied.text} (copy)`
  await expect(receipts(canvasElement)).toEqual(['copy outbox/attempt/unknown-reply', sent, sent])
  // Recorded: the composer closes and says where the copy's outcome lives, without claiming delivery.
  await expect(list.querySelector('[data-outbox-copy-notice]')).toHaveTextContent('The copy is saved as its own new message. If Fractal does not confirm it, it is listed here. Do not send it again.')
  await expect(list.querySelector('[data-outbox-copy-notice]')).toHaveAttribute('role', 'status')
}

const notAppliedPlay: Story['play'] = async ({ canvasElement }) => {
  const list = await region(canvasElement)
  const attempt = row(canvasElement, 'outbox/attempt/not-applied')
  await expect(within(attempt).getByText('Fractal did not apply this message.')).toBeVisible()
  await expect(within(attempt).getByText('Fractal refused it: the conversation was closed.')).toBeVisible()
  await expect(rowActions(attempt)).toEqual(['Retry', 'Discard'])
  await expectFits(canvasElement, list)
  await keepThroughConfirmation(canvasElement, 'outbox/attempt/not-applied', 'This removes the message from this device. It will not be sent.')
  await userEvent.click(within(attempt).getByRole('button', { name: /^Retry/ }))
  await waitFor(() => expect(within(canvasElement).queryByRole('region', { name: RECOVERY_TITLE })).toBeNull())
  await expect(receipts(canvasElement)).toEqual(['retry outbox/attempt/not-applied'])
}

const pendingPlay: Story['play'] = async ({ canvasElement }) => {
  const list = await region(canvasElement)
  const start = row(canvasElement, 'outbox/attempt/pending-start')
  await expect(within(start).getByText('This thread has not been sent.')).toBeVisible()
  for (const id of ['outbox/attempt/pending-start', 'outbox/attempt/pending-reply']) await expect(rowActions(row(canvasElement, id))).toEqual(['Revalidate and send', 'Discard'])
  await expectFits(canvasElement, list)
  await userEvent.click(within(start).getByRole('button', { name: /^Revalidate and send/ }))
  await waitFor(() => expect(canvasElement.querySelector('[data-outbox-item="outbox/attempt/pending-start"]')).toBeNull())
  await expect(receipts(canvasElement)).toEqual(['send outbox/attempt/pending-start'])
  await expect(row(canvasElement, 'outbox/attempt/pending-reply')).toBeVisible()
}

const corruptPlay: Story['play'] = async ({ canvasElement }) => {
  const list = await region(canvasElement)
  const entry = row(canvasElement, 'outbox/attempt/corrupt')
  await expect(within(entry).getByText('This outbox entry is damaged and cannot be sent.')).toBeVisible()
  await expect(rowActions(entry)).toEqual(['Discard'])
  await expectFits(canvasElement, list)
  await userEvent.click(within(entry).getByRole('button', { name: /^Discard/ }))
  const dialog = await within(canvasElement).findByRole('alertdialog', { name: 'Discard from this device?' })
  await expect(dialog).toHaveAccessibleDescription('This removes the entry from this device. It does not cancel anything Fractal may already have received.')
  await userEvent.click(within(dialog).getByRole('button', { name: 'Discard' }))
  await waitFor(() => expect(within(canvasElement).queryByRole('region', { name: RECOVERY_TITLE })).toBeNull())
  await expect(receipts(canvasElement)).toEqual(['discard outbox/attempt/corrupt'])
}

const identityMismatchPlay: Story['play'] = async ({ canvasElement }) => {
  const list = await region(canvasElement)
  const entry = row(canvasElement, 'outbox/attempt/foreign')
  await expect(within(entry).getByText('This entry belongs to a different Fractal identity and cannot be sent from here.')).toBeVisible()
  await expect(rowActions(entry)).toEqual(['Discard'])
  await expectFits(canvasElement, list)
}

const unavailablePlay: Story['play'] = async ({ canvasElement }) => {
  const notice = await region(canvasElement)
  await expect(within(notice).getByRole('status')).toHaveTextContent(RECOVERY_UNAVAILABLE)
  await expect(within(notice).queryAllByRole('button')).toEqual([])
  await expect(notice.textContent?.toLowerCase()).not.toMatch(/damaged|corrupt/)
  await expectFits(canvasElement, notice)
}

export const Unknown: Story = { tags: ['critical'], play: unknownPlay }
export const NotApplied: Story = { args: { state: 'not-applied' }, play: notAppliedPlay }
export const Pending: Story = { args: { state: 'pending' }, play: pendingPlay }
export const Corrupt: Story = { args: { state: 'corrupt' }, play: corruptPlay }
export const IdentityMismatch: Story = { args: { state: 'identity-mismatch' }, play: identityMismatchPlay }
export const Unavailable: Story = { args: { state: 'unavailable' }, play: unavailablePlay }

/** Visit every state in one canvas without duplicating the recovery landmark: only the selected tab's list is mounted. */
export const AllStates: Story = {
  render: args => <main>
    <Tabs defaultSelectedKey="unknown" {...stylex.props(styles.tabs)}>
      <TabList aria-label="Outbox recovery states" {...stylex.props(styles.tabList)}>
        {states.map(state => <Tab key={state} id={state} {...stylex.props(styles.tab)}>{stateLabel[state]}</Tab>)}
      </TabList>
      {states.map(state => <TabPanel key={state} id={state}>
        <RecoveryStory key={`${state}:${args.scheme}`} scheme={args.scheme} state={state} />
      </TabPanel>)}
    </Tabs>
  </main>,
  play: async context => {
    const canvas = within(context.canvasElement)
    const plays: Readonly<Record<State, Story['play']>> = {
      unknown: unknownPlay, 'not-applied': notAppliedPlay, pending: pendingPlay, corrupt: corruptPlay, 'identity-mismatch': identityMismatchPlay, unavailable: unavailablePlay,
    }
    for (const state of states) {
      await userEvent.click(canvas.getByRole('tab', { name: stateLabel[state] }))
      await expect(canvas.getByRole('tabpanel', { name: stateLabel[state] })).toBeVisible()
      await plays[state]?.(context)
      await expect(canvas.getAllByRole('main')).toHaveLength(1)
      await expect(canvas.queryAllByRole('region', { name: RECOVERY_TITLE }).length).toBeLessThanOrEqual(1)
    }
  },
}

const styles = stylex.create({
  canvas: { minHeight: '100vh', boxSizing: 'border-box', paddingBlock: s.section, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  log: { maxWidth: geometryVars.lane, boxSizing: 'border-box', marginInline: 'auto', marginTop: s.lg, paddingInline: s.lg, color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  receipt: { margin: 0 },
  tabs: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  tabList: { display: 'flex', flexWrap: 'wrap', gap: s.xs, padding: s.md, backgroundColor: surface.canvas },
  tab: { paddingBlock: s.xs, paddingInline: s.md, borderRadius: r.sm, color: { default: ink.fgMuted, ':is([data-selected])': ink.fg }, backgroundColor: { default: 'transparent', ':is([data-selected])': surface.rowActive }, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', outlineStyle: { default: 'none', ':is([data-focus-visible])': 'solid' }, outlineWidth: geometryVars.focusRing, outlineColor: accent.primary },
})
