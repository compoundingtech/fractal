/** An Inbox item's first-send recipient: the real LiveInboxThreads reading a local exact-key start preview decoded by the generated schema. */
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import * as Native from '@fractal/client/schema/read'
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import * as Aria from 'react-aria-components'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import startRefusalWire from '../../../../docs/st3/client-v0/fixtures/thread-start-refusals.json'
import { colorVars as c, geometryVars as g, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { Markdown } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Markdown.tsx'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { observed, unavailable, waiting, type Feed } from '../data/source.ts'
import { LiveInboxThreads } from './LiveThreads.tsx'
import type { ThreadDraft, ThreadSource } from './threadSource.ts'

const actor = 'person/operator'
const original = 'agent/build-host-a/worker-tests'
const configured = 'agent/build-host-c/triage-desk'
const fallback = 'agent/build-host-b/release-desk'
// A typed system notice: no agent asked it and no agent is listed on it; only the preview names a recipient.
const sourceId = 'system/notice/release-window'
const episode = Native.decodeUnknownSync(Native.Revision)('claim/start-recipient/release-window')
const existingId = 'thread/release-checklist'

type Scenario = 'original' | 'configured' | 'fallback' | 'mismatch' | 'unconfigured' | 'offline' | 'loading' | 'failed' | 'no-preview' | 'repeat' | 'existing-reply'
type Args = {
  readonly scheme: 'dark' | 'light'
  /** Desktop Inbox detail at 1440px, or the phone layout at 390px. */
  readonly viewport: 1440 | 390
  readonly scenario: Scenario
}

const fallbackBasis = { _tag: 'fallback', original_agent_id: original, retirement: { claim_id: 'claim/start-recipient/retired', kind: 'agent.retired', state: 'retired' } }
/** Every preview goes through the generated decoder, exactly as the gateway source decodes Fractal's answer. */
const context = (agent_id: string, basis: object): Feed<Native.ThreadStartContext> => observed({
  value: Native.decodeUnknownSync(Native.ThreadStartContext)({ kind: 'thread-start-context', source_id: sourceId, episode, start_recipient: { agent_id, basis } }),
})
const originalContext = context(original, { _tag: 'original' })
const fallbackContext = context(fallback, fallbackBasis)
const unassignedRefusal = startRefusalWire['recipient-unassigned']
/** What the exact-key preview first answers in each scenario. */
const initialPreview = (scenario: Scenario): Feed<Native.ThreadStartContext> => {
  switch (scenario) {
    case 'configured': return context(configured, { _tag: 'configured' })
    case 'fallback': case 'existing-reply': return fallbackContext
    case 'unconfigured': return unavailable({ reason: 'failed', code: unassignedRefusal.code, detail: 'Fractal could not load the first-send recipient.', explanation: unassignedRefusal.message })
    case 'loading': return waiting
    case 'failed': return unavailable({ reason: 'failed', detail: 'Fractal could not load the first-send recipient.', explanation: 'The gateway did not answer in time.' })
    case 'original': case 'mismatch': case 'offline': case 'no-preview': case 'repeat': return originalContext
  }
}

/** A local thread source: the preview, one optional existing thread, and first sends a recipient change refuses once. */
const createFixture = (scenario: Scenario) => {
  const registry = AtomRegistry.make()
  const now = Date.parse('2026-11-17T10:00:00Z')
  const agent = (id: string, name: string, offline: boolean) => Native.decodeUnknownSync(Native.Agent)({
    kind: 'agent', id, name, host_id: `host/${id.split('/')[1]}`, driver: 'omp', state: offline ? 'stopped' : 'waiting',
    blocked_on: null, fault: null, reason: offline ? 'Stopped; its host is unreachable.' : 'Waiting for follow-up.', runtime_ids: [], reachability: offline ? 'unreachable' : 'reachable',
    revision: '1', updated_at: new Date(now).toISOString(),
  })
  const alert = Native.decodeUnknownSync(Native.Attention)({
    id: 'attention/start-recipient', kind: 'attention', revision: episode, episode, person_id: actor, source_id: sourceId,
    requester_id: actor, attention_kind: 'custom.release.notice', title: 'Release window opens at 14:00',
    detail: 'The release window opens at 14:00 UTC. The checklist must be complete before then.', priority: 'high', state: 'open', alert: true, actions: ['work.done'],
    requested_at: new Date(now - 25 * 60_000).toISOString(), updated_at: new Date(now).toISOString(),
  })
  const summary = (id: string, title: string) => Native.decodeUnknownSync(Native.ThreadSummary)({
    id, title, state: 'open', agent_id: original, title_revision: 'claim/title', revision: 'claim/current', message_count: 1, unread: false,
  })
  const existing = summary(existingId, 'Release window opens at 14:00')
  const history = [Native.decodeUnknownSync(Native.ThreadMessageEvent)({
    kind: 'message', id: 'message/checklist', actor_id: original, accepted_at_unix_ms: now - 30 * 60_000,
    content: 'The checklist is drafted. Rollback steps still need a second look.', thread_quote_id: null,
  })]
  const preview = Atom.keepAlive(Atom.make(initialPreview(scenario)))
  const drafts = Atom.family((_destination: string) => Atom.keepAlive(Atom.make<ThreadDraft>({ content: '' })))
  let sends = 0
  const noChange = { detail: 'This story does not change threads.' }
  const threads: ThreadSource = {
    access: Atom.make({ read: true, inboxOrigins: true, actor, actions: new Set(['thread.start', 'thread.reply']) }),
    titles: Atom.make(new Map([[existingId, existing.title]])),
    list: () => Atom.make(observed({ value: { items: scenario === 'existing-reply' ? [existing] : [], next_cursor: Option.none() } })),
    detail: id => Atom.make(observed({ value: { summary: id === existingId ? existing : summary(id, `Started thread ${id.split('-').at(-1)}`), snapshotId: 'snapshot/start-recipient' } })),
    timeline: () => Atom.make(observed({ value: { items: history, next_cursor: Option.none() } })),
    draft: drafts,
    // A source without the preview reader names no recipient at all.
    ...(scenario === 'no-preview' ? {} : {
      startContext: () => preview,
      reloadStartContext: () => {},
    }),
    start: async () => {
      // The daemon selected a new recipient after this preview: nothing is appended and the preview is read again.
      if (scenario === 'mismatch' && sends++ === 0) {
        registry.set(preview, fallbackContext)
        return { _tag: 'Refused', detail: "Couldn't start this thread.", explanation: 'The requested recipient is unavailable.' }
      }
      // Every Start is its own discussion, even on the same source episode.
      if (scenario === 'repeat') return { _tag: 'Sent', threadId: `thread/started-${++sends}` }
      return { _tag: 'Refused', detail: 'This story does not send messages.' }
    },
    reply: async () => ({ _tag: 'Refused', detail: 'This story does not send messages.' }),
    rename: async () => noChange,
    changeState: async () => noChange,
  }
  const source = { ...fixtureSource({ world: {
    now, events: [], missions: [], attention: [alert],
    agents: [agent(original, 'Worker tests', scenario === 'offline'), agent(configured, 'Triage desk', false), agent(fallback, 'Release desk', false)],
    conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } }), threads }
  return { registry, source, alert }
}

const StartRecipientStory = ({ scheme, viewport, scenario }: Args) => {
  const [fixture] = React.useState(() => createFixture(scenario))
  React.useEffect(() => () => fixture.registry.dispose(), [fixture])
  return <main data-scheme={scheme} data-viewport={viewport} style={{ width: viewport }}
    {...stylex.props(styles.canvas, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
    <NarrowLayoutContext.Provider value={viewport < 900}>
      <DataSourceProvider source={fixture.source} registry={fixture.registry}>
        <div data-start-recipient-scene={scenario} {...stylex.props(styles.pane)}>
          <h2 {...stylex.props(styles.title)}>{fixture.alert.title}</h2>
          <div {...stylex.props(styles.detail)}><Markdown text={fixture.alert.detail} /></div>
          <LiveInboxThreads alert={fixture.alert} />
        </div>
      </DataSourceProvider>
    </NarrowLayoutContext.Provider>
  </main>
}

const scenarios: readonly Scenario[] = ['original', 'configured', 'fallback', 'mismatch', 'unconfigured', 'offline', 'loading', 'failed', 'no-preview', 'repeat', 'existing-reply']
const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/Threads/Start recipient',
  component: StartRecipientStory,
  parameters: {
    layout: 'fullscreen',
    scenario: { _tag: 'contract-fixture', reason: 'Exact-key thread start previews decoded by the generated ThreadStartContext schema (original, configured, terminal-retirement fallback, recipient-retired refusal, loading, failure, no reader) and a one-time recipient mismatch served by a local source; no world slice models start recipients. No daemon writes.' },
    docs: { description: { component: 'Before a first send any Inbox item names its exact recipient: the agent, its exact id and the basis Fractal selected it on. A fallback is labelled New recipient with the retired original agent and id on their own line; an unconfigured source, with or without an original agent, refuses with no default; a source without the preview offers no Start. When Fractal selects a different recipient after the draft opened, the draft and refusal stay and the person sends again explicitly.' } },
  },
  args: { scheme: 'dark', viewport: 1440, scenario: 'original' },
  argTypes: {
    scheme: { options: ['dark', 'light'], control: 'inline-radio' },
    viewport: { options: [1440, 390], control: 'inline-radio' },
    scenario: { options: scenarios, control: 'select' },
  },
  render: args => <StartRecipientStory key={`${args.scenario}:${args.viewport}:${args.scheme}`} {...args} />,
} satisfies Meta<Args>
export default meta

type Story = StoryObj<Args>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStates:contract-fixture',
  'Configured:contract-fixture',
  'ExistingReply:contract-fixture',
  'Fallback:contract-fixture',
  'FallbackLight:contract-fixture',
  'FallbackNarrow:contract-fixture',
  'FallbackNarrowLight:contract-fixture',
  'Loading:contract-fixture',
  'MismatchReread:contract-fixture',
  'NoPreviewReader:contract-fixture',
  'OfflineNormal:contract-fixture',
  'Original:contract-fixture',
  'PreviewFailed:contract-fixture',
  'RepeatedStarts:contract-fixture',
  'UnconfiguredRefusal:contract-fixture',
]

const openDraft = async (canvasElement: HTMLElement) => {
  const canvas = within(canvasElement)
  await userEvent.click(await canvas.findByRole('button', { name: 'Start thread' }))
  return canvas
}
const recipientRow = (canvasElement: HTMLElement, basis: string) => canvasElement.querySelector(`[data-start-recipient="${basis}"]`)

/** The item's own agent receives the first send: its exact id and basis are shown, nothing is labelled new. */
export const Original: Story = {
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(recipientRow(canvasElement, 'original')?.textContent).toContain('Ask Worker tests about this item without answering it.'))
    await openDraft(canvasElement)
    const recipient = recipientRow(canvasElement, 'original')
    await expect(recipient?.textContent).toContain('To Worker tests')
    await expect(recipient?.querySelector('[data-start-recipient-id]')?.textContent).toBe(original)
    await expect(recipient?.querySelector('[data-start-recipient-basis]')?.textContent).toBe('Original recipient')
    await expect(canvasElement.querySelector('[data-start-recipient-original-row]')).toBeNull()
  },
}

/** A configured recipient for an item no agent is listed on. */
export const Configured: Story = {
  args: { scenario: 'configured' },
  play: async ({ canvasElement }) => {
    await openDraft(canvasElement)
    const recipient = recipientRow(canvasElement, 'configured')
    await expect(recipient?.textContent).toContain('To Triage desk')
    await expect(recipient?.querySelector('[data-start-recipient-id]')?.textContent).toBe(configured)
    await expect(recipient?.querySelector('[data-start-recipient-basis]')?.textContent).toBe('Configured recipient')
  },
}

/** Terminal retirement selects the configured fallback: the new recipient is labelled, the original agent and id stay named. */
const fallbackPlay: Story['play'] = async ({ canvasElement }) => {
  await openDraft(canvasElement)
  const recipient = recipientRow(canvasElement, 'fallback')
  await expect(recipient?.textContent).toContain('To Release desk')
  await expect(recipient?.querySelector('[data-start-recipient-id]')?.textContent).toBe(fallback)
  await expect(recipient?.querySelector('[data-start-recipient-basis]')?.textContent).toBe('New recipient')
  await expect(recipient?.querySelector('[data-start-recipient-original-row]')?.textContent).toContain('Originally for Worker tests')
  await expect(recipient?.querySelector('[data-start-recipient-original]')?.textContent).toBe(original)
  await expect(canvasElement.querySelector('textarea')).toBeVisible()
}
export const Fallback: Story = { args: { scenario: 'fallback' }, play: fallbackPlay }
export const FallbackLight: Story = { args: { scenario: 'fallback', scheme: 'light' }, play: fallbackPlay }
export const FallbackNarrow: Story = { args: { scenario: 'fallback', viewport: 390 }, play: fallbackPlay }
export const FallbackNarrowLight: Story = { args: { scenario: 'fallback', viewport: 390, scheme: 'light' }, play: fallbackPlay }

/** Fractal refused a first send whose recipient changed: the draft and refusal remain, and only a second Send starts it. */
export const MismatchReread: Story = {
  args: { scenario: 'mismatch' },
  play: async ({ canvasElement }) => {
    const canvas = await openDraft(canvasElement)
    await userEvent.type(canvasElement.querySelector('textarea')!, 'Are the rollback steps covered?')
    await userEvent.click(canvas.getByRole('button', { name: 'Send' }))
    await waitFor(() => expect(canvasElement.querySelector('[data-start-recipient-changed]')).not.toBeNull())
    await expect(canvasElement.querySelector('[data-start-recipient-changed]')?.textContent).toContain('The recipient changed to Release desk.')
    await expect(recipientRow(canvasElement, 'fallback')?.querySelector('[data-start-recipient-id]')?.textContent).toBe(fallback)
    await expect(canvasElement.querySelector('[role="alert"]')?.textContent).toContain("Couldn't start this thread.")
    await expect(canvasElement.querySelector('textarea')?.value).toBe('Are the rollback steps covered?')
  },
}

/** An unconfigured source, including one with no original agent: Fractal's refusal, no Start and no default recipient. */
export const UnconfiguredRefusal: Story = {
  args: { scenario: 'unconfigured' },
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(canvasElement.querySelector('[data-start-recipient="unassigned"]')).not.toBeNull())
    await expect(canvasElement.querySelector('[data-start-recipient="unassigned"]')?.textContent).toContain('No recipient is configured. Set up a responsible agent before starting a thread.')
    await expect(canvasElement.querySelector('[data-start-recipient="unassigned"]')?.textContent).toContain(unassignedRefusal.message)
    await expect(canvasElement.querySelector('[data-start-recipient="retired"]')).toBeNull()
    await expect(within(canvasElement).queryByRole('button', { name: 'Start thread' })).toBeNull()
    await expect(canvasElement.querySelector('[data-start-recipient-id]')).toBeNull()
    await expect(canvasElement.querySelector('textarea')).toBeNull()
  },
}

/** An offline, stopped agent is still the recipient: availability is never read as retirement. */
export const OfflineNormal: Story = {
  args: { scenario: 'offline' },
  play: async ({ canvasElement }) => {
    await openDraft(canvasElement)
    await expect(recipientRow(canvasElement, 'original')?.querySelector('[data-start-recipient-id]')?.textContent).toBe(original)
    await expect(canvasElement.querySelector('[data-start-recipient="fallback"]')).toBeNull()
  },
}

/** While the preview loads there is no recipient and no Start. */
export const Loading: Story = {
  args: { scenario: 'loading' },
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(canvasElement.textContent).toContain('Loading the recipient'))
    await expect(within(canvasElement).queryByRole('button', { name: 'Start thread' })).toBeNull()
  },
}

/** A failed preview offers Retry and nothing to send to. */
export const PreviewFailed: Story = {
  args: { scenario: 'failed' },
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(canvasElement.textContent).toContain('Fractal could not load the first-send recipient.'))
    await expect(within(canvasElement).queryByRole('button', { name: 'Start thread' })).toBeNull()
  },
}

/** A source without the preview reader never guesses a recipient. */
export const NoPreviewReader: Story = {
  args: { scenario: 'no-preview' },
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(canvasElement.querySelector('[data-start-recipient="none"]')).not.toBeNull())
    await expect(within(canvasElement).queryByRole('button', { name: 'Start thread' })).toBeNull()
  },
}

/** Each Start is its own discussion: after one opens, Start is offered again for a second. */
export const RepeatedStarts: Story = {
  args: { scenario: 'repeat' },
  play: async ({ canvasElement }) => {
    const canvas = await openDraft(canvasElement)
    await userEvent.type(canvasElement.querySelector('textarea')!, 'Is the checklist complete?')
    await userEvent.click(canvas.getByRole('button', { name: 'Send' }))
    await canvas.findByRole('button', { name: 'Start thread' })
  },
}

/** An existing thread keeps replying to its stored agent; only a new first send previews the fallback. */
export const ExistingReply: Story = {
  args: { scenario: 'existing-reply' },
  play: async ({ canvasElement }) => {
    const panel = await waitFor(() => {
      const found = canvasElement.querySelector<HTMLElement>(`[data-thread-panel][aria-label="Release window opens at 14:00"]`)
      if (found === null) throw new Error('The existing thread is not listed yet')
      return found
    })
    await userEvent.click(panel.querySelector<HTMLButtonElement>('button[aria-expanded]')!)
    await waitFor(() => expect(panel.querySelector('textarea')).not.toBeNull())
    await expect(panel.querySelector('[data-start-recipient]')).toBeNull()
    await expect(panel.textContent).toContain('Rollback steps still need a second look.')
  },
}

/** Compare first-send contracts while mounting only the selected Inbox detail. */
export const AllStates: Story = {
  render: args => <Aria.Tabs aria-label="Start recipient scenarios" defaultSelectedKey="original">
    <Aria.TabList aria-label="Scenarios">
      {scenarios.map(scenario => <Aria.Tab key={scenario} id={scenario}>{scenario}</Aria.Tab>)}
    </Aria.TabList>
    {scenarios.map(scenario => <Aria.TabPanel key={scenario} id={scenario}>
      <StartRecipientStory {...args} scenario={scenario} viewport={390} />
    </Aria.TabPanel>)}
  </Aria.Tabs>,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    for (const scenario of scenarios) {
      await userEvent.click(canvas.getByRole('tab', { name: scenario }))
      await expect(canvas.getAllByRole('main')).toHaveLength(1)
      await expect(canvasElement.querySelectorAll('[data-start-recipient-scene]')).toHaveLength(1)
      await expect(canvasElement.querySelector(`[data-start-recipient-scene="${scenario}"]`)).toBeVisible()
    }
  },
}

const styles = stylex.create({
  canvas: { minHeight: '100vh', boxSizing: 'border-box', padding: s.section, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize },
  // The Inbox detail pane's own measure: full width on a phone, the reading lane on a desktop.
  pane: { display: 'flex', flexDirection: 'column', gap: s.sm, maxWidth: g.lane, width: '100%', marginInline: 'auto', minWidth: 0 },
  title: { margin: 0, color: c.fg, fontSize: t.uiSize, fontWeight: 600 },
  detail: { margin: 0, color: c.fgMuted, fontSize: t.metaSize },
})
