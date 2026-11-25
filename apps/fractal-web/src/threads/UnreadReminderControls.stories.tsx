/** The person's unread reminder in a thread heading: the real ThreadPanel and UnreadReminderControls over a local port. */
import * as Native from '@fractal/client/schema/read'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import type { Meta, StoryObj } from '@storybook/react'
import { RegistryContext } from '@effect/atom-react'
import * as stylex from '@stylexjs/stylex'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { colorVars as c, geometryVars as g, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { ThreadPanel } from './ThreadPanel.tsx'
import { UnreadReminderControls } from './UnreadReminderControls.tsx'
import type { UnreadActionPort, UnreadActionResult, UnreadReminderRead, UnreadSeekResult } from './unread-action-port.ts'
import { threadAnchorAttribute, threadAnchorSelector } from './threadAnchor.ts'

const threadId = 'thread/worker-tests'
const agent = 'agent/build-host-a/worker-tests'
const person = 'person/operator'
/** Oldest first, as the thread timeline renders them. Every readable message can anchor a reminder. */
const replies = [
  { id: 'message/retry-ready', actor: agent, content: 'The retry fix is ready. I will run the worker tests before merge.' },
  { id: 'message/cancellation-ask', actor: person, content: 'Can you verify the cancellation case too?' },
  { id: 'message/cancellation-fenced', actor: agent, content: 'Cancellation keeps the original attempt fenced. Covering the retry case in the same run.' },
  { id: 'message/linux-pass', actor: agent, content: 'Worker tests pass on Linux.' },
  { id: 'message/macos-running', actor: agent, content: 'The macOS job is still running; I will post its result here.' },
  { id: 'message/review-ready', actor: agent, content: 'All worker tests pass. PR #42 is ready for review.' },
] as const
const remindedAnchor = 'message/cancellation-fenced'
const reminded = Native.decodeUnknownSync(Native.ThreadUnreadReminder)({ anchor_message_id: remindedAnchor, marks: [{ mark_id: 'operation/mark-story-1', anchor_message_id: remindedAnchor }] })

type Args = {
  readonly scheme: 'dark' | 'light'
  /** What the authoritative reminder read reports for this thread. */
  readonly read: 'reminded' | 'none' | 'unavailable'
  /** Whether this connection holds the mark-unread and clear-unread grant. */
  readonly grant: boolean
  /** What Fractal answers when Jump seeks the reminded reply. */
  readonly seek: 'current' | 'pending' | 'unavailable' | 'refused'
  /** Whether Fractal accepts Mark read and Mark unread. */
  readonly change: 'accept' | 'refuse'
}

/**
 * Copy for each seek outcome, matching the gateway port's own sentences. `refused` is how the port reports a seek
 * Fractal refused or could not answer: an unavailable reply, never a body or a guessed row.
 */
const seekResults: Record<Args['seek'], UnreadSeekResult> = {
  current: { _tag: 'Found', messageId: remindedAnchor },
  pending: { _tag: 'Pending', detail: 'This reminder’s message has not arrived yet.' },
  unavailable: { _tag: 'Unavailable', detail: 'This reminder’s message is no longer available.' },
  refused: { _tag: 'Unavailable', detail: 'Fractal could not load this reminder’s message. Try again once it reconnects.' },
}
const reads: Record<Args['read'], UnreadReminderRead> = {
  reminded: { _tag: 'Known', reminder: reminded },
  none: { _tag: 'Known', reminder: undefined },
  unavailable: { _tag: 'Unavailable', detail: 'Reconnect and load this thread before using unread reminders.' },
}
const refusedChange: UnreadActionResult = { _tag: 'Refused', detail: 'Fractal refused this unread reminder change.' }

/** A local port over one fixture atom: marks and clears update the read the controls observe. No daemon, no writes. */
const createFixture = ({ read, grant, seek, change }: Omit<Args, 'scheme'>) => {
  const registry = AtomRegistry.make()
  const reminder = Atom.keepAlive(Atom.make<UnreadReminderRead>(reads[read]))
  let next = 1
  const port: UnreadActionPort = {
    canMark: grant,
    reminder: () => reminder,
    mark: async ({ anchor_message_id }) => {
      if (change === 'refuse') return refusedChange
      const mark_id = `operation/mark-story-${++next}`
      registry.set(reminder, { _tag: 'Known', reminder: Native.decodeUnknownSync(Native.ThreadUnreadReminder)({ anchor_message_id, marks: [{ mark_id, anchor_message_id }] }) })
      return { _tag: 'Sent' }
    },
    clear: async ({ observed_marks }) => {
      if (change === 'refuse') return refusedChange
      const current = registry.get(reminder)
      if (current._tag === 'Known' && current.reminder !== undefined) {
        const marks = current.reminder.marks.filter(mark => !observed_marks.includes(mark.mark_id))
        registry.set(reminder, { _tag: 'Known', reminder: marks.length === 0 ? undefined : { ...current.reminder, marks } })
      }
      return { _tag: 'Sent' }
    },
    seek: async () => seekResults[seek],
  }
  return { registry, port }
}

/** Replies whose text is inside the scrolled timeline, in timeline order, as the production viewport capture reports them. */
const visibleIn = (timeline: HTMLElement | null) => (): readonly string[] => {
  if (timeline === null) return []
  const frame = timeline.getBoundingClientRect()
  return [...timeline.querySelectorAll<HTMLElement>(threadAnchorSelector)].filter(row => {
    const rect = row.getBoundingClientRect()
    return rect.bottom > frame.top && rect.top < frame.bottom
  }).map(row => row.getAttribute(threadAnchorAttribute)!)
}

const ReminderScene = ({ label, ...args }: Omit<Args, 'scheme'> & { readonly label?: string }) => {
  const [fixture] = React.useState(() => createFixture(args))
  React.useEffect(() => () => fixture.registry.dispose(), [fixture])
  const [timeline, setTimeline] = React.useState<HTMLOListElement | null>(null)
  // Open at the newest reply, as a reader returning to the thread would, so the reminded reply starts out of view.
  React.useLayoutEffect(() => { if (timeline !== null) timeline.scrollTop = timeline.scrollHeight }, [timeline])
  return <RegistryContext.Provider value={fixture.registry}>
    <div data-reminder-scene={label ?? ''} {...stylex.props(styles.scene)}>
      {label === undefined ? null : <p {...stylex.props(styles.label)}>{label}</p>}
      <ThreadPanel thread={{ title: 'Worker tests', state: 'open' }} messageCount={replies.length} unread initiallyExpanded
        onMarkRead={async () => undefined} canMarkRead
        unreadControls={<UnreadReminderControls threadId={threadId} port={fixture.port} visibleReplies={visibleIn(timeline)} timeline={timeline} />}>
        <ol ref={setTimeline} aria-label="Worker tests replies" tabIndex={0} {...stylex.props(styles.timeline)}>
          {replies.map(reply => <li key={reply.id} {...{ [threadAnchorAttribute]: reply.id }} {...stylex.props(styles.reply)}>
            <span {...stylex.props(styles.author)}>{reply.actor === agent ? 'Worker tests' : 'You'}</span>
            <span data-thread-message-content="">{reply.content}</span>
          </li>)}
        </ol>
      </ThreadPanel>
    </div>
  </RegistryContext.Provider>
}

const Canvas = ({ scheme, children }: { readonly scheme: Args['scheme']; readonly children: React.ReactNode }) =>
  <main data-scheme={scheme} {...stylex.props(styles.canvas, scheme === 'dark' ? styles.darkScheme : styles.lightScheme, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>{children}</main>

const UnreadReminderStory = ({ scheme, ...args }: Args) =>
  <Canvas scheme={scheme}><ReminderScene key={`${args.read}:${args.grant}:${args.seek}:${args.change}`} {...args} /></Canvas>

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/Threads/Unread reminder',
  component: UnreadReminderStory,
  parameters: {
    layout: 'fullscreen',
    scenario: { _tag: 'contract-fixture', reason: 'The production thread heading and reminder controls over a local unread port whose marks and clears update one fixture read; scenario slices carry no unread reminder. No daemon writes.' },
    docs: { description: { component: 'The person\'s own unread reminder, kept apart from natural unread: the indicator and Jump when Fractal reports one, Mark read and Mark unread only with the reminder grant, and nothing when the read is unavailable. Jump answers come from the fixture port: a loaded reply, a pending one, or an unavailable one, each without inventing a message body.' } },
  },
  args: { scheme: 'dark', read: 'reminded', grant: true, seek: 'current', change: 'accept' },
  argTypes: {
    scheme: { options: ['dark', 'light'], control: 'inline-radio' },
    read: { options: ['reminded', 'none', 'unavailable'], control: 'inline-radio' },
    grant: { control: 'boolean' },
    seek: { options: ['current', 'pending', 'unavailable', 'refused'], control: 'inline-radio' },
    change: { options: ['accept', 'refuse'], control: 'inline-radio' },
  },
} satisfies Meta<Args>
export default meta

type Story = StoryObj<Args>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStates:contract-fixture',
  'JumpCurrent:contract-fixture',
  'JumpPending:contract-fixture',
  'JumpRefused:contract-fixture',
  'JumpUnavailable:contract-fixture',
  'Light:contract-fixture',
  'NoReminder:contract-fixture',
  'ReadOnly:contract-fixture',
  'ReadUnavailable:contract-fixture',
  'RefusedChange:contract-fixture',
  'Reminded:contract-fixture',
]

const markUnread = 'Mark unread from the first visible reply'
const markRead = 'Mark read, clearing the unread reminder'
const jump = 'Jump to the reply marked unread'
const controls = (canvasElement: HTMLElement) => canvasElement.querySelector('[data-thread-unread-reminder-controls]')
const indicators = (canvasElement: HTMLElement) => canvasElement.querySelectorAll('[data-thread-unread-reminder]')
/** These are text actions on the themed heading, not native buttons with a browser-default face. */
const expectHeadingActions = async (canvasElement: HTMLElement) => {
  const themedCanvas = canvasElement.querySelector<HTMLElement>('main[data-scheme]')!
  await expect(getComputedStyle(themedCanvas).colorScheme).toBe(themedCanvas.dataset.scheme)
  const canvas = within(canvasElement)
  const naturalRead = await canvas.findAllByRole('button', { name: 'Mark visible replies read' })
  const reminderActions = canvasElement.querySelectorAll<HTMLButtonElement>('[data-thread-unread-reminder-controls] button')
  for (const button of [...naturalRead, ...reminderActions]) {
    await userEvent.unhover(button)
    const heading = button.closest<HTMLElement>('[data-thread-read-heading]')!.getBoundingClientRect()
    const bounds = button.getBoundingClientRect()
    await expect(bounds.left).toBeGreaterThanOrEqual(heading.left)
    await expect(bounds.right).toBeLessThanOrEqual(heading.right)
    await expect(getComputedStyle(button).backgroundColor).toBe('rgba(0, 0, 0, 0)')
    await expect(getComputedStyle(button).borderTopWidth).toBe('0px')
  }
}
const rowInView = (canvasElement: HTMLElement, id: string) => {
  const timeline = canvasElement.querySelector<HTMLElement>('ol[aria-label="Worker tests replies"]')!
  const row = [...timeline.querySelectorAll<HTMLElement>(threadAnchorSelector)].find(row => row.getAttribute(threadAnchorAttribute) === id)!
  const frame = timeline.getBoundingClientRect(), rect = row.getBoundingClientRect()
  return rect.top >= frame.top - 1 && rect.bottom <= frame.bottom + 1
}
/** Jump answered without moving the view: only the reason shows, the reminder and the loaded replies are unchanged. */
const jumpStops = (detail: string): NonNullable<Story['play']> => async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expectHeadingActions(canvasElement)
  await userEvent.click(await canvas.findByRole('button', { name: jump }))
  await expect(await canvas.findByRole('status')).toHaveTextContent(detail)
  await expectHeadingActions(canvasElement)
  await expect(canvas.queryByRole('alert')).toBeNull()
  await expect(indicators(canvasElement)).toHaveLength(1)
  await expect(canvasElement.querySelectorAll(threadAnchorSelector)).toHaveLength(replies.length)
  await expect(rowInView(canvasElement, remindedAnchor)).toBe(false)
}

/** With the grant, Mark read clears exactly the observed reminder and Mark unread sets a new one from the first visible reply. */
export const Reminded: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expectHeadingActions(canvasElement)
    const indicator = canvasElement.querySelector<HTMLElement>('[data-thread-unread-reminder]')!
    await expect(indicator).toBeVisible()
    await expect(indicator).toHaveTextContent('Marked unread')
    await expect(canvas.getByRole('button', { name: jump })).toBeEnabled()
    // Natural read stays its own action beside the reminder.
    await expect(canvas.getByRole('button', { name: 'Mark visible replies read' })).toBeEnabled()
    await userEvent.click(canvas.getByRole('button', { name: markRead }))
    await waitFor(() => expect(indicators(canvasElement)).toHaveLength(0))
    await expect(canvas.queryByRole('button', { name: jump })).toBeNull()
    await expect(canvas.queryByRole('button', { name: markRead })).toBeNull()
    await userEvent.click(canvas.getByRole('button', { name: markUnread }))
    await waitFor(() => expect(indicators(canvasElement)).toHaveLength(1))
    await expect(canvas.getByRole('button', { name: markRead })).toBeEnabled()
    await expect(canvas.queryByRole('alert')).toBeNull()
  },
}
export const Light: Story = { args: { scheme: 'light' }, play: Reminded.play }

/** No reminder yet: no indicator and no Jump, only Mark unread for a granted connection. */
export const NoReminder: Story = {
  args: { read: 'none' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(await canvas.findByRole('button', { name: markUnread })).toBeEnabled()
    await expect(indicators(canvasElement)).toHaveLength(0)
    await expect(canvas.queryByRole('button', { name: jump })).toBeNull()
    await expect(canvas.queryByRole('button', { name: markRead })).toBeNull()
  },
}

/** Without the grant the reminder still shows with Jump, but nothing can change it. */
export const ReadOnly: Story = {
  args: { grant: false },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expectHeadingActions(canvasElement)
    const indicator = canvasElement.querySelector<HTMLElement>('[data-thread-unread-reminder]')!
    await expect(indicator).toBeVisible()
    await expect(indicator).toHaveTextContent('Marked unread')
    await expect(canvas.getByRole('button', { name: jump })).toBeEnabled()
    await expect(canvas.queryByRole('button', { name: markUnread })).toBeNull()
    await expect(canvas.queryByRole('button', { name: markRead })).toBeNull()
  },
}

/** An unavailable read is not evidence either way: no indicator and no mutation, even with the grant. */
export const ReadUnavailable: Story = {
  args: { read: 'unavailable' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(await canvas.findByRole('button', { name: 'Mark visible replies read' })).toBeEnabled()
    await expect(controls(canvasElement)).toBeNull()
  },
}

/** A current reply: Jump reveals the reminded row that is already loaded, with no notice. */
export const JumpCurrent: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(rowInView(canvasElement, remindedAnchor)).toBe(false)
    await userEvent.click(await canvas.findByRole('button', { name: jump }))
    await waitFor(() => expect(rowInView(canvasElement, remindedAnchor)).toBe(true))
    await expect(canvas.queryByRole('status')).toBeNull()
    await expect(indicators(canvasElement)).toHaveLength(1)
  },
}
/** Pending: the signed reply has not arrived, so Jump names that and neither adds a row nor moves the view. */
export const JumpPending: Story = { args: { seek: 'pending' }, play: jumpStops('This reminder’s message has not arrived yet.') }
export const JumpUnavailable: Story = { args: { seek: 'unavailable' }, play: jumpStops('This reminder’s message is no longer available.') }
/** A refused or failed seek reaches the reader as the port's generic unavailable reason. */
export const JumpRefused: Story = { args: { seek: 'refused' }, play: jumpStops('Fractal could not load this reminder’s message. Try again once it reconnects.') }

/** Fractal refuses the change: one alert with its reason, and the reminder stays as Fractal last reported it. */
export const RefusedChange: Story = {
  args: { change: 'refuse' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: markRead }))
    await expect(await canvas.findByRole('alert')).toHaveTextContent('Fractal refused this unread reminder change.')
    await expect(indicators(canvasElement)).toHaveLength(1)
    await expect(canvas.getByRole('button', { name: markRead })).toBeEnabled()
  },
}

const allStates: ReadonlyArray<Omit<Args, 'scheme'> & { readonly label: string }> = [
  { label: 'Reminded, granted', read: 'reminded', grant: true, seek: 'current', change: 'accept' },
  { label: 'No reminder, granted', read: 'none', grant: true, seek: 'current', change: 'accept' },
  { label: 'Reminded, read-only', read: 'reminded', grant: false, seek: 'current', change: 'accept' },
  { label: 'Read unavailable', read: 'unavailable', grant: true, seek: 'current', change: 'accept' },
  { label: 'Jump: pending reply', read: 'reminded', grant: true, seek: 'pending', change: 'accept' },
  { label: 'Jump: unavailable reply', read: 'reminded', grant: true, seek: 'unavailable', change: 'accept' },
  { label: 'Jump: refused seek', read: 'reminded', grant: true, seek: 'refused', change: 'accept' },
  { label: 'Change refused', read: 'reminded', grant: true, seek: 'current', change: 'refuse' },
]

/** Every reminder state side by side, each with its own registry and port. */
export const AllStates: Story = {
  render: ({ scheme }) => <Canvas scheme={scheme}>
    <div {...stylex.props(styles.grid)}>{allStates.map(({ label, ...state }) => <ReminderScene key={label} label={label} {...state} />)}</div>
  </Canvas>,
  play: async ({ canvasElement }) => {
    const scene = (label: string) => canvasElement.querySelector<HTMLElement>(`[data-reminder-scene="${label}"]`)!
    await waitFor(() => expect(canvasElement.querySelectorAll('[data-thread-panel]')).toHaveLength(allStates.length))
    await expectHeadingActions(canvasElement)
    await expect(indicators(scene('Reminded, granted'))).toHaveLength(1)
    await expect(indicators(scene('No reminder, granted'))).toHaveLength(0)
    await expect(within(scene('Reminded, read-only')).queryByRole('button', { name: markRead })).toBeNull()
    await expect(controls(scene('Read unavailable'))).toBeNull()
  },
}

const styles = stylex.create({
  canvas: { minHeight: '100vh', boxSizing: 'border-box', padding: s.section, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize },
  darkScheme: { colorScheme: 'dark' },
  lightScheme: { colorScheme: 'light' },
  scene: { display: 'flex', flexDirection: 'column', gap: s.sm, maxWidth: g.lane, width: '100%', marginInline: 'auto', minWidth: 0 },
  grid: { display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(min(100%, 360px), 1fr))', gap: s.section },
  label: { margin: 0, color: c.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  // Short enough that the reminded reply starts out of view at every width the play gate runs.
  timeline: { listStyle: 'none', margin: 0, padding: s.sm, display: 'flex', flexDirection: 'column', gap: s.sm, height: 120, overflowY: 'auto' },
  reply: { display: 'flex', flexDirection: 'column', gap: s.xs, fontSize: t.uiSize, lineHeight: t.metaLeading },
  author: { color: c.fgMuted, fontSize: t.metaSize },
})
