/** The Inbox's owner-wide marked-unread section: the real MarkedUnreadThreads paging a local canonical list source. */
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import * as Native from '@fractal/client/schema/read'
import type { Meta, StoryObj } from '@storybook/react'
import { RegistryContext } from '@effect/atom-react'
import * as stylex from '@stylexjs/stylex'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { Tab, TabList, TabPanel, Tabs } from 'react-aria-components'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { colorVars as c, geometryVars as g, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { observed, unavailable, waiting, type Feed } from '../data/source.ts'
import { MarkedUnreadThreads, type MarkedUnreadSource } from './MarkedUnreadThreads.tsx'
import type { UnreadActionPort, UnreadReminderRead, UnreadSeekResult } from './unread-action-port.ts'

const agent = 'agent/build-host-a/worker-tests'
const summary = (id: string, title: string, state: 'open' | 'closed', count: number, unread: boolean, anchor: string | null) =>
  Native.decodeUnknownSync(Native.ThreadSummary)({
    id, title, state, agent_id: agent, title_revision: `claim/${id}/title`, revision: `claim/${id}/current`, message_count: count, unread,
    unread_reminder: anchor === null ? null : { anchor_message_id: anchor, marks: [{ mark_id: `operation/${id}/mark`, anchor_message_id: anchor }] },
  })
/** Natural unread and the private reminder are independent: the first is naturally unread, the others are not. */
const review = summary('thread/review-pr-42', 'Review PR #42', 'open', 4, true, 'message/review-ready')
const cancellation = summary('thread/cancellation', 'Cancellation behavior', 'open', 2, false, 'message/cancellation-retry')
const rollout = summary('thread/rollout-notes', 'Rollout notes', 'closed', 3, false, 'message/rollout-plan')
/** Its reminder was cleared before the collection dropped it: the page still carries it, the list must not. */
const cleared = summary('thread/release-checklist', 'Release checklist', 'open', 1, true, null)
/** What seeking each reminder's anchor answers, with the gateway port's own sentences. No message body is supplied. */
const seeks: Readonly<Record<string, UnreadSeekResult>> = {
  [review.id]: { _tag: 'Found', messageId: 'message/review-ready' },
  [cancellation.id]: { _tag: 'Pending', detail: 'This reminder’s message has not arrived yet.' },
  [rollout.id]: { _tag: 'Unavailable', detail: 'This reminder’s message is no longer available.' },
}

type Scenario = 'rows' | 'empty' | 'empty-page-with-cursor' | 'loading' | 'fetch-failed' | 'repeated-cursor' | 'page-bound'
type Args = {
  readonly scheme: 'dark' | 'light'
  /** The canonical `marked_unread` pages the source serves, cursor by cursor. */
  readonly scenario: Scenario
}

const firstPage = ''
const page = (items: readonly Native.ThreadSummary[], next?: string): Feed<Native.ThreadSummaryPage> =>
  observed({ value: { items, next_cursor: next === undefined ? Option.none() : Option.some(next) } })
/** Every scenario's pages, keyed by the cursor that requests them; the first page has no cursor. */
const pagesFor = (scenario: Scenario, failing: boolean) => (cursor: string): Feed<Native.ThreadSummaryPage> => {
  switch (scenario) {
    case 'rows': return cursor === firstPage ? page([review], 'cursor/page-2') : page([cancellation, cleared, rollout])
    case 'empty': return page([])
    // An empty page that still names a cursor is not an empty Inbox; the next page holds the reminder.
    case 'empty-page-with-cursor': return cursor === firstPage ? page([], 'cursor/page-2') : page([rollout])
    case 'loading': return waiting
    case 'fetch-failed': return cursor === firstPage ? page([review], 'cursor/page-2')
      : failing ? unavailable({ reason: 'failed', detail: 'Fractal could not load threads.' }) : page([cancellation, rollout])
    case 'repeated-cursor': return cursor === firstPage ? page([review], 'cursor/page-2') : page([], 'cursor/page-2')
    // Every page names a fresh cursor, so the reader stops at its page bound instead of paging forever.
    case 'page-bound': return page([], `cursor/page-${cursor === firstPage ? 2 : Number(cursor.slice('cursor/page-'.length)) + 1}`)
  }
}

/** A local source over the canonical list contract. Opening only seeks; nothing marks, clears or writes. */
const createFixture = (scenario: Scenario) => {
  const registry = AtomRegistry.make()
  const failing = Atom.keepAlive(Atom.make(scenario === 'fetch-failed'))
  const pages = Atom.family((cursor: string) => Atom.make(get => pagesFor(scenario, get(failing))(cursor)))
  const noChange = { _tag: 'Unavailable', detail: 'The marked-unread list does not change reminders.' } as const
  const unreadActions: UnreadActionPort = {
    canMark: false,
    reminder: () => Atom.make<UnreadReminderRead>({ _tag: 'Unavailable', detail: 'The marked-unread list does not read thread detail.' }),
    mark: async () => noChange,
    clear: async () => noChange,
    seek: async ({ threadId }) => seeks[threadId] ?? { _tag: 'Unavailable', detail: 'Fractal could not locate this reminder’s message.' },
  }
  const source: MarkedUnreadSource = {
    list: query => pages(query.cursor ?? firstPage),
    unreadActions,
    reload: () => registry.set(failing, false),
  }
  return { registry, source }
}

const MarkedUnreadScene = ({ scenario }: { readonly scenario: Scenario }) => {
  const [fixture] = React.useState(() => createFixture(scenario))
  React.useEffect(() => () => fixture.registry.dispose(), [fixture])
  const [opened, setOpened] = React.useState<string>()
  const title = [review, cancellation, rollout].find(thread => thread.id === opened)?.title
  return <RegistryContext.Provider value={fixture.registry}>
    <div data-marked-unread-scene={scenario} {...stylex.props(styles.scene)}>
      <MarkedUnreadThreads source={fixture.source} onOpenThread={setOpened} />
      {/* Story harness only: stands in for the workspace navigation the Inbox passes as onOpenThread. */}
      {title === undefined ? null : <p data-story-opened={opened} {...stylex.props(styles.label)}>Story navigation: opened “{title}” at the reply marked unread.</p>}
    </div>
  </RegistryContext.Provider>
}

const Canvas = ({ scheme, children }: { readonly scheme: Args['scheme']; readonly children: React.ReactNode }) =>
  <main data-scheme={scheme} {...stylex.props(styles.canvas, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>{children}</main>

const MarkedUnreadStory = ({ scheme, scenario }: Args) => <Canvas scheme={scheme}><MarkedUnreadScene key={scenario} scenario={scenario} /></Canvas>

const scenarios: readonly Scenario[] = ['rows', 'empty', 'empty-page-with-cursor', 'loading', 'fetch-failed', 'repeated-cursor', 'page-bound']
const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/Threads/Marked unread',
  component: MarkedUnreadStory,
  parameters: {
    layout: 'fullscreen',
    scenario: { _tag: 'contract-fixture', reason: 'Canonical marked_unread list pages served cursor by cursor from a local source, including empty pages with cursors, repeated cursors and an endless cursor chain; scenario slices carry no unread reminders. Opening seeks only; no daemon writes.' },
    docs: { description: { component: 'The person\'s private marked-unread threads across every page of the canonical list, closed threads included. Natural unread keeps its own dot; the private reminder is named in the age column. Opening seeks the reminder\'s reply first: a current reply opens the thread, a pending or unavailable one stays in the list with the reason.' } },
  },
  args: { scheme: 'dark', scenario: 'rows' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'inline-radio' }, scenario: { options: scenarios, control: 'select' } },
} satisfies Meta<Args>
export default meta

type Story = StoryObj<Args>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStates:contract-fixture',
  'Empty:contract-fixture',
  'EmptyPageWithCursor:contract-fixture',
  'FetchFailed:contract-fixture',
  'Light:contract-fixture',
  'Loading:contract-fixture',
  'PageBoundExceeded:contract-fixture',
  'RepeatedCursor:contract-fixture',
  'Rows:contract-fixture',
]

const row = (canvasElement: HTMLElement, id: string) => canvasElement.querySelector<HTMLElement>(`[data-marked-unread-thread="${id}"]`)
const rows = (canvasElement: HTMLElement) => canvasElement.querySelectorAll('[data-marked-unread-thread]')
const expectError = (tag: string, message: string): NonNullable<Story['play']> => async ({ canvasElement }) => {
  const error = await waitFor(() => {
    const element = canvasElement.querySelector<HTMLElement>(`[data-marked-unread-error="${tag}"]`)
    if (element === null) throw new Error(`no ${tag} notice yet`)
    return element
  })
  await expect(error).toHaveTextContent(message)
  // A list that could not be completed shows no partial rows and never claims to be empty.
  await expect(rows(canvasElement)).toHaveLength(0)
  await expect(within(canvasElement).queryByText('No marked-unread threads')).toBeNull()
}

/** Every page is read, closed threads stay listed, and opening seeks before it navigates. */
export const Rows: Story = {
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(rows(canvasElement)).toHaveLength(3))
    // The second page's cleared row is skipped; the rollout row after it on that page still shows.
    await expect(row(canvasElement, cleared.id)).toBeNull()
    for (const thread of [review, cancellation, rollout]) {
      await expect(row(canvasElement, thread.id)!.querySelector('[data-private-unread-reminder]')).toBeVisible()
      await expect(within(row(canvasElement, thread.id)!).getByText('Private reminder')).toBeInTheDocument()
    }
    await expect(row(canvasElement, rollout.id)).toHaveTextContent('Closed')
    // Natural unread is independent of the reminder: only the naturally unread thread has the dot.
    await expect(row(canvasElement, review.id)).toHaveTextContent('Unread,')
    await expect(row(canvasElement, cancellation.id)).not.toHaveTextContent('Unread,')

    await userEvent.click(within(row(canvasElement, cancellation.id)!).getByRole('button'))
    await expect(await within(row(canvasElement, cancellation.id)!).findByText('This reminder’s message has not arrived yet.')).toBeVisible()
    await userEvent.click(within(row(canvasElement, rollout.id)!).getByRole('button'))
    await expect(await within(row(canvasElement, rollout.id)!).findByText('This reminder’s message is no longer available.')).toBeVisible()
    await expect(canvasElement.querySelector('[data-story-opened]')).toBeNull()
    await expect(rows(canvasElement)).toHaveLength(3)

    await userEvent.click(within(row(canvasElement, review.id)!).getByRole('button'))
    await waitFor(() => expect(canvasElement.querySelector('[data-story-opened]')?.getAttribute('data-story-opened')).toBe(review.id))
  },
}
export const Light: Story = { args: { scheme: 'light' }, play: Rows.play }

/** Only a terminal page establishes that nothing is marked unread. */
export const Empty: Story = {
  args: { scenario: 'empty' },
  play: async ({ canvasElement }) => {
    await expect(await within(canvasElement).findByText('No marked-unread threads')).toBeVisible()
    await expect(rows(canvasElement)).toHaveLength(0)
  },
}

/** An empty first page with a cursor keeps paging; the reminder on the next page is listed and no empty state shows. */
export const EmptyPageWithCursor: Story = {
  args: { scenario: 'empty-page-with-cursor' },
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(rows(canvasElement)).toHaveLength(1))
    await expect(row(canvasElement, rollout.id)).toHaveTextContent('Rollout notes')
    await expect(within(canvasElement).queryByText('No marked-unread threads')).toBeNull()
  },
}

export const Loading: Story = {
  args: { scenario: 'loading' },
  play: async ({ canvasElement }) => {
    await expect(await within(canvasElement).findByText('Loading marked-unread threads')).toBeVisible()
    await expect(rows(canvasElement)).toHaveLength(0)
  },
}

/** A later page fails: the whole list reports it, and Retry reads every page again. */
export const FetchFailed: Story = {
  args: { scenario: 'fetch-failed' },
  play: async context => {
    const message = 'Couldn\'t load marked-unread threads. Fractal could not load threads.'
    await expectError('FetchFailed', message)(context)
    await userEvent.click(within(context.canvasElement).getByRole('button', { name: `Retry: ${message}` }))
    await waitFor(() => expect(rows(context.canvasElement)).toHaveLength(3))
    await expect(context.canvasElement.querySelector('[data-marked-unread-error]')).toBeNull()
  },
}
export const RepeatedCursor: Story = {
  args: { scenario: 'repeated-cursor' },
  play: expectError('RepeatedCursor', 'Couldn\'t finish loading marked-unread threads because Fractal repeated a page. Try again.'),
}
export const PageBoundExceeded: Story = {
  args: { scenario: 'page-bound' },
  play: expectError('PageBoundExceeded', 'Couldn\'t finish loading marked-unread threads because the page limit was reached. Try again.'),
}

const allStates: ReadonlyArray<{ readonly scenario: Scenario; readonly label: string }> = [
  { scenario: 'rows', label: 'Rows across two pages' },
  { scenario: 'empty', label: 'Empty after the last page' },
  { scenario: 'empty-page-with-cursor', label: 'Empty page with a cursor' },
  { scenario: 'loading', label: 'Loading' },
  { scenario: 'fetch-failed', label: 'Page failed' },
  { scenario: 'repeated-cursor', label: 'Repeated cursor' },
  { scenario: 'page-bound', label: 'Page bound reached' },
]

/** Visit every owner-wide list state with only the selected Inbox section mounted. */
export const AllStates: Story = {
  render: ({ scheme }) => <Canvas scheme={scheme}>
    <Tabs defaultSelectedKey="rows">
      <TabList aria-label="Marked-unread thread states">
        {allStates.map(({ scenario, label }) => <Tab key={scenario} id={scenario}>{label}</Tab>)}
      </TabList>
      {allStates.map(({ scenario }) => <TabPanel key={scenario} id={scenario}>
        <MarkedUnreadScene key={scenario} scenario={scenario} />
      </TabPanel>)}
    </Tabs>
  </Canvas>,
  play: async context => {
    const canvas = within(context.canvasElement)
    const stateStories: Record<Scenario, Story> = {
      rows: Rows,
      empty: Empty,
      'empty-page-with-cursor': EmptyPageWithCursor,
      loading: Loading,
      'fetch-failed': FetchFailed,
      'repeated-cursor': RepeatedCursor,
      'page-bound': PageBoundExceeded,
    }
    for (const { scenario, label } of allStates) {
      await userEvent.click(canvas.getByRole('tab', { name: label }))
      await expect(canvas.getByRole('tabpanel', { name: label })).toBeVisible()
      await stateStories[scenario].play?.(context)
      await expect(canvas.getAllByRole('region', { name: 'Marked unread' })).toHaveLength(1)
      // Count DOM nodes too: hidden or retained panels must not keep another list mounted.
      await expect(context.canvasElement.querySelectorAll('[data-marked-unread-threads]')).toHaveLength(1)
      await expect(context.canvasElement.querySelectorAll('[data-marked-unread-scene]')).toHaveLength(1)
    }
    // React Aria owns keyboard selection as well as the tab/panel relationships.
    await userEvent.keyboard('{Home}')
    await expect(canvas.getByRole('tab', { name: 'Rows across two pages' })).toHaveAttribute('aria-selected', 'true')
    await expect(canvas.getByRole('tabpanel', { name: 'Rows across two pages' })).toBeVisible()
    await waitFor(() => expect(rows(context.canvasElement)).toHaveLength(3))
    await expect(context.canvasElement.querySelectorAll('[data-marked-unread-threads]')).toHaveLength(1)
  },
}

const styles = stylex.create({
  canvas: { minHeight: '100vh', boxSizing: 'border-box', padding: s.section, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize },
  scene: { display: 'flex', flexDirection: 'column', gap: s.sm, maxWidth: g.lane, width: '100%', marginInline: 'auto', minWidth: 0 },
  label: { margin: 0, color: c.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
})
