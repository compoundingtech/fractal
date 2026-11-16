/** The Alerts tab's Auto-resolved section over literal asks, in a fixed 256px sidebar, overridden against a stubbed port. */
import type { AutoResolvedAsk } from '@fractal/client'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { colorVars as c, geometryVars as g, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import type { AlertActionResult, AutoResolvedRead } from '../conversation/alerts/alert-action-port.ts'
import { observed } from '../data/source.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { AlertsList } from './AlertsTab.tsx'

const REPLY_MS = 400
const COPY = 'Your answer replaces the default. Work already done on the default is not undone; example/atlas is told.'

type Args = {
  readonly scheme: 'dark' | 'light'
  /** How the stubbed port answers an override: accepted (the row leaves on the next read), refused, or never. */
  readonly reply: 'sent' | 'refused' | 'pending'
  /** The override window closed on every ask, so none offers one. */
  readonly expired?: boolean
  /** The read lists nothing but may be missing asks (it reached its row cap). */
  readonly emptyIncomplete?: boolean
}

/** Asks a default answered at their deadline, an hour and a day ago. */
const asks = (now: number, expired: boolean): readonly AutoResolvedAsk[] => [
  {
    resolution_id: 'a'.repeat(64), title: 'Merge the parser change?', requester_id: 'agent/example/atlas',
    resolved_at: new Date(now - 60 * 60_000).toISOString(), target_id: 'step-run/release/parser', episode: 'claim/example-parser',
    default_answer: { id: 'hold', label: 'Hold' },
    request: {
      version: 1, type: 'decision', question: 'Merge the parser change?', why_person: 'You own the release.',
      answers: [
        { id: 'merge', label: 'Merge', consequence: 'It joins the merge queue.' },
        { id: 'hold', label: 'Hold', consequence: 'The builder waits.' },
        { id: 'drop', label: 'Drop', consequence: 'The change is closed.' },
      ],
    },
    ...(expired ? {} : { action_parameters: { 'work.override': { target_id: 'step-run/release/parser', episode: 'claim/example-parser' } } }),
  },
  {
    resolution_id: 'b'.repeat(64), title: 'Rotate the staging credentials now?', requester_id: 'agent/example/sentinel',
    resolved_at: new Date(now - 26 * 60 * 60_000).toISOString(), target_id: 'step-run/ops/rotate', episode: 'claim/example-rotate',
    default_answer: { id: 'later', label: 'Tonight' },
    request: {
      version: 1, type: 'decision', question: 'Rotate the staging credentials now?', why_person: 'Rotation logs everyone out.',
      answers: [
        { id: 'now', label: 'Now', consequence: 'Sessions end at once.' },
        { id: 'later', label: 'Tonight', consequence: 'Rotation runs at 02:00.' },
        { id: 'replan', label: 'Change the plan', consequence: 'Sentinel replans the rotation from your words.', outcome: 'request_changes' },
      ],
    },
    ...(expired ? {} : { action_parameters: { 'work.override': { target_id: 'step-run/ops/rotate', episode: 'claim/example-rotate' } } }),
  },
]

const Story = ({ scheme, reply, expired = false, emptyIncomplete = false }: Args) => {
  const [now] = React.useState(Date.now)
  const [read, setRead] = React.useState<AutoResolvedRead>(() => emptyIncomplete
    ? { _tag: 'Observed', asks: [], complete: false }
    : { _tag: 'Observed', asks: asks(now, expired), complete: true })
  // The port answers after a moment; an accepted override leaves the next read, as Fractal's does.
  const onOverride = (ask: AutoResolvedAsk) => {
    const { promise, resolve } = Promise.withResolvers<AlertActionResult>()
    if (reply !== 'pending') setTimeout(() => {
      if (reply === 'refused') return resolve({ _tag: 'Refused', detail: 'The override window closed, so the default stands.' })
      resolve({ _tag: 'Sent' })
      setRead((prior) => prior._tag === 'Observed' ? { ...prior, asks: prior.asks.filter((each) => each.resolution_id !== ask.resolution_id) } : prior)
    }, REPLY_MS)
    return promise
  }
  return (
    <NarrowLayoutContext.Provider value={window.innerWidth < 900}>
      <main data-scheme={scheme} {...stylex.props(styles.canvas, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
        <aside aria-label="Alerts sidebar" {...stylex.props(styles.sidebar)}>
          <AlertsList feed={observed({ value: [] })} now={now} onOpen={() => {}} autoResolved={read} onOverride={onOverride} />
        </aside>
      </main>
    </NarrowLayoutContext.Provider>
  )
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/Sidebar/Alerts tab/Auto-resolved',
  component: Story,
  parameters: {
    layout: 'fullscreen',
    scenario: { _tag: 'contract-fixture', reason: 'Literal auto-resolved asks from the client-v0 auto-resolved read; scenario worlds serve no deadline-resolved asks yet.' },
    docs: { description: { component: 'Asks a default answered at their deadline in the last seven days, below the open alerts: title, how long ago, the default that applied and who asked. While Fractal offers the override, the ask\'s other named answers are one click each, above the consequence every client shows verbatim; a request for changes opens a field and sends once it has the person\'s words. The answer shows as on its way at once and the row\'s answers hold until it settles; a refusal is said in words and the answers stay usable; an accepted override leaves on the next read. An ask whose window closed shows only what happened; a read that may be missing asks says so, even when it lists none.' } },
  },
  args: { scheme: 'dark', reply: 'sent' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, reply: { options: ['sent', 'refused', 'pending'], control: 'radio' }, expired: { control: 'boolean' }, emptyIncomplete: { control: 'boolean' } },
} satisfies Meta<Args>
export default meta

type S = StoryObj<Args>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'EmptyIncomplete:contract-fixture',
  'Expired:contract-fixture',
  'Overridable:contract-fixture',
  'Pending:contract-fixture',
  'Refused:contract-fixture',
  'RequestChanges:contract-fixture',
]

const section = (canvasElement: HTMLElement) => within(within(canvasElement).getByRole('region', { name: 'Auto-resolved' }))

/** One click on another answer overrides the default; the row says so at once and leaves on the next read. */
export const Overridable: S = {
  tags: ['critical'],
  play: async ({ canvasElement }) => {
    const rows = section(canvasElement)
    // The section is laid out inside the sidebar, not clipped to nothing.
    const region = within(canvasElement).getByRole('region', { name: 'Auto-resolved' }).getBoundingClientRect()
    await expect(region.height).toBeGreaterThan(0)
    await expect(region.bottom).toBeLessThanOrEqual(within(canvasElement).getByRole('complementary', { name: 'Alerts sidebar' }).getBoundingClientRect().bottom)
    await expect(rows.getByText('Default applied: Hold · example/atlas')).toBeVisible()
    await expect(rows.getAllByText(COPY)[0]).toBeVisible()
    const group = rows.getByRole('group', { name: 'Answer “Merge the parser change?” instead' })
    // The default is never offered as its own override, and every answer is reachable from the keyboard.
    await expect(within(group).getAllByRole('button').map((button) => button.textContent)).toEqual(['Merge', 'Drop'])
    await userEvent.tab()
    await expect(within(group).getByRole('button', { name: 'Merge' })).toHaveFocus()
    await userEvent.keyboard('{Enter}')
    await expect(rows.getByRole('status')).toHaveTextContent('Sending “Merge”…')
    await waitFor(() => expect(rows.queryByText('Merge the parser change?')).toBeNull())
    // Focus stays in the section, on the next ask.
    await expect(within(rows.getByRole('group', { name: 'Answer “Rotate the staging credentials now?” instead' })).getByRole('button', { name: 'Now' })).toHaveFocus()
  },
}

/** The window closed: the row says what the default was, and offers nothing. */
export const Expired: S = {
  args: { expired: true, scheme: 'light' },
  play: async ({ canvasElement }) => {
    const rows = section(canvasElement)
    await expect(rows.getByText('Default applied: Tonight · example/sentinel')).toBeVisible()
    await expect(rows.queryAllByRole('button')).toHaveLength(0)
    await expect(rows.queryByText(COPY)).toBeNull()
  },
}

/** An override on its way: the row says so, and its answers hold so another click sends nothing more. */
export const Pending: S = {
  args: { reply: 'pending' },
  play: async ({ canvasElement }) => {
    const rows = section(canvasElement)
    await userEvent.click(rows.getByRole('button', { name: 'Drop' }))
    await expect(rows.getByRole('status')).toHaveTextContent('Sending “Drop”…')
    await expect(rows.getByRole('button', { name: 'Merge' })).toHaveAttribute('aria-disabled', 'true')
    await expect(rows.getByRole('button', { name: 'Drop' })).toHaveAttribute('aria-disabled', 'true')
  },
}

/** Fractal refused the override: the reason in words, and the answers are usable again. */
export const Refused: S = {
  args: { reply: 'refused' },
  play: async ({ canvasElement }) => {
    const rows = section(canvasElement)
    await userEvent.click(rows.getByRole('button', { name: 'Merge' }))
    await expect(await rows.findByRole('alert')).toHaveTextContent('The override window closed, so the default stands.')
    await expect(rows.getByRole('button', { name: 'Drop' })).not.toHaveAttribute('aria-disabled')
  },
}

/** A request for changes carries the person's words: its answer opens a field, and Send waits for words. */
export const RequestChanges: S = {
  args: { reply: 'pending' },
  play: async ({ canvasElement }) => {
    const rows = section(canvasElement)
    const replan = rows.getByRole('button', { name: 'Change the plan' })
    await userEvent.click(replan)
    await expect(replan).toHaveAttribute('aria-expanded', 'true')
    const words = rows.getByRole('textbox', { name: 'What should change, for “Change the plan”' })
    await expect(words).toHaveFocus()
    const send = rows.getByRole('button', { name: 'Send “Change the plan”' })
    await expect(send).toBeDisabled()
    await userEvent.type(words, '   ')
    await expect(send).toBeDisabled()
    await userEvent.type(words, 'Rotate after the Friday demo.')
    await expect(send).toBeEnabled()
    await userEvent.click(send)
    await expect(rows.getByRole('status')).toHaveTextContent('Sending “Change the plan”…')
  },
}

/** A read that lists nothing but may be missing asks keeps the section, and says so. */
export const EmptyIncomplete: S = {
  args: { emptyIncomplete: true },
  play: async ({ canvasElement }) => {
    const rows = section(canvasElement)
    await expect(rows.getByText('Older auto-resolved asks may be missing from this list.')).toBeVisible()
    await expect(rows.queryByRole('list')).toBeNull()
  },
}

const styles = stylex.create({
  canvas: { display: 'flex', alignItems: 'flex-start', minHeight: '100vh', boxSizing: 'border-box', padding: 12, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize },
  sidebar: { width: g.sidebarDefault, flexShrink: 0, boxSizing: 'border-box', backgroundColor: c.sidebar, border: `1px solid ${c.border}` },
})
