// Real-browser proofs for the virtual transcript's follow mode and member anchors.
// Each play asserts after the re-render and carries its own negative control.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, waitFor, within } from 'storybook/test'
import { Transcript, type TranscriptHistory, type TranscriptTurn } from './assistant-ui/composition/Transcript'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import type { ConversationItem, TextItem } from './assistant-ui/embrace-data/model'
import { workLogTurnFromItems } from './assistant-ui/taste/work-log'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const at = '2026-01-15T12:00:00Z'
const now = Date.parse(at)
const prose = (label: string, count: number) => Array.from({ length: count }, (_, index) => `${label}, note ${index + 1}: Compare the retained selection with the recorded input before changing the projection.`).join('\n\n')
const answer = (id: string, text: string): ConversationItem => ({ _tag: 'Text', id, role: 'assistant', text, attachments: [], streaming: false, at })
const read = (id: string, path: string): ConversationItem => ({ _tag: 'ToolCall', id, callId: id, name: 'read', input: { path }, status: 'success', callSeen: true, at, result: { content: `export const source = '${path}'\n`, isError: false, at } })
const ask = (id: string, text: string): TextItem & { role: 'user' } => ({ _tag: 'Text', id, role: 'user', attachments: [], streaming: false, at, text })
const turnOf = (id: string, items: readonly ConversationItem[], prompt?: TextItem & { role: 'user' }): TranscriptTurn => ({
  id, prompt, items,
  work: workLogTurnFromItems(items, { kindFor: () => 'read', running: false, failed: false, interrupted: false, durationMs: 12000, startedAt: at, completeHistory: true }),
})
const checkpoint = (index: number) => turnOf(`checkpoint/${index}`, [read(`checkpoint/${index}/read`, `src/checkpoint-${index}.ts`), answer(`checkpoint/${index}/answer`, prose(`Checkpoint ${index}`, 1 + index % 4))], ask(`checkpoint/${index}/prompt`, `Review checkpoint ${index}.`))
const range = (from: number, count: number) => Array.from({ length: count }, (_, index) => checkpoint(from + index))

// A leading partial turn: history starts mid-turn, so the turn is named after its first item.
// Backfill reveals its prompt and an earlier tool, renaming the turn, and older turns above it.
const leadItems: readonly ConversationItem[] = [
  read('lead/tool-a', 'src/lead.ts'),
  { _tag: 'Reasoning', id: 'lead/thinking', text: 'Check the selection before the projection changes.', streaming: false, at },
  answer('lead/answer', prose('Leading answer', 6)),
]
const lead = turnOf('lead/tool-a', leadItems)
const backfilledLead = turnOf('lead/prompt', [read('lead/earlier', 'src/earlier.ts'), ...leadItems], ask('lead/prompt', 'Compare the leading selection with the recorded input.'))
const splitBefore = [lead, ...range(100, 10)]
const splitAfter = [...range(90, 6), backfilledLead, ...range(100, 10)]
const asyncTurns = range(0, 12)

type Scenario = 'split' | 'async' | 'gesture'
const styles = stylex.create({
  root: { width: g.polishWide, maxWidth: '100%', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  narrow: { width: g.polishNarrow },
  controls: { display: 'flex', flexWrap: 'wrap', gap: s.md, paddingBlock: s.md },
  frame: { height: g.transcriptFrameHeight },
  pair: { display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(min(100%, 360px), 1fr))', gap: s.lg },
})

function Conversation({ label, turns, history, context }: { label: string; turns: readonly TranscriptTurn[]; history: TranscriptHistory; context?: string }) {
  const messages = React.useMemo(() => turns.flatMap(turn => [...(turn.prompt === undefined ? [] : [turn.prompt]), ...turn.items]), [turns])
  return <section aria-label={label} {...stylex.props(styles.frame)}>
    <EmbraceRuntimeProvider options={{ messages, onNew: async () => {} }}>
      <Transcript title={label} turns={turns} history={history} landmarkContext={context} sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} onOpenTool={() => {}} />
    </EmbraceRuntimeProvider>
  </section>
}

function GestureLane({ older }: { older: boolean }) {
  const [requests, setRequests] = React.useState(0)
  const [grown, setGrown] = React.useState(false)
  const label = older ? 'Older history available' : 'Complete history'
  const history: TranscriptHistory = older ? { _tag: 'More', loadOlder: async () => { setRequests(value => value + 1) } } : { _tag: 'Complete' }
  const turns = React.useMemo(() => grown ? range(200, 8) : range(200, 1), [grown])
  return <div data-gesture-lane={older ? 'older' : 'complete'} data-requests={requests}>
    <div {...stylex.props(styles.controls)}>
      <button type="button" disabled={grown} onMouseDown={event => event.preventDefault()} onClick={() => setGrown(true)}>Append replies to {label.toLowerCase()}</button>
    </div>
    <Conversation label={label} turns={turns} history={history} context={label} />
  </div>
}

function FollowAnchor({ scheme = 'dark', narrow = false, scenario = 'split' }: { scheme?: Scheme; narrow?: boolean; scenario?: Scenario }) {
  const [backfilled, setBackfilled] = React.useState(false)
  const [requests, setRequests] = React.useState(0)
  const splitHistory: TranscriptHistory = backfilled ? { _tag: 'Complete' } : { _tag: 'More', loadOlder: async () => { setRequests(value => value + 1) } }
  return <main data-scheme={scheme} data-requests={requests} {...stylex.props(styles.root, narrow && styles.narrow, ...baselineTheme, scheme === 'light' && lightTheme)}>
    {scenario === 'split' && <>
      <div {...stylex.props(styles.controls)}>
        <button type="button" disabled={backfilled} onMouseDown={event => event.preventDefault()} onClick={() => setBackfilled(true)}>Deliver earlier prompt</button>
      </div>
      <Conversation label="Leading turn backfill" turns={backfilled ? splitAfter : splitBefore} history={splitHistory} />
    </>}
    {scenario === 'async' && <Conversation label="Asynchronous layout" turns={asyncTurns} history={{ _tag: 'Complete' }} />}
    {scenario === 'gesture' && <div {...stylex.props(styles.pair)}><GestureLane older={false} /><GestureLane older /></div>}
  </main>
}

const meta = {
  title: 'Fractal UI/Transcript follow and anchors',
  component: FollowAnchor,
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark', narrow: false, scenario: 'split' },
  argTypes: { scheme: { control: 'radio', options: ['light', 'dark'] }, narrow: { control: 'boolean' }, scenario: { control: 'radio', options: ['split', 'async', 'gesture'] } },
} satisfies Meta<typeof FollowAnchor>
export default meta
type Story = StoryObj<typeof meta>

const frames = (count = 3) => new Promise<void>(resolve => {
  const step = (left: number) => left === 0 ? resolve() : requestAnimationFrame(() => step(left - 1))
  step(count)
})
/** Frames for resize observation and RAC relayout, then a quiet period past the reader-input window. */
const settle = async () => {
  await frames(4)
  await new Promise<void>(resolve => setTimeout(resolve, 350))
  await frames(2)
}
const gap = (lane: HTMLElement) => lane.scrollHeight - lane.scrollTop - lane.clientHeight
const offset = (lane: HTMLElement, element: Element) => element.getBoundingClientRect().top - lane.getBoundingClientRect().top
const entry = (lane: HTMLElement, id: string) => [...lane.querySelectorAll<HTMLElement>('[data-embrace-entry-id]')].find(candidate => candidate.dataset['embraceEntryId'] === id && candidate.getBoundingClientRect().height > 0)
const summary = (lane: HTMLElement, turn: string) => [...lane.querySelectorAll<HTMLElement>('[data-scroll-anchor-id]')].find(candidate => candidate.dataset['scrollAnchorId'] === JSON.stringify(['work-summary', turn]))
/** A reader gesture, then the scroll it produces. */
const readerScroll = async (lane: HTMLElement, top: number) => {
  lane.dispatchEvent(new WheelEvent('wheel', { deltaY: top < lane.scrollTop ? -120 : 120, bubbles: true }))
  lane.scrollTop = top
  await settle()
}

async function exerciseSplit({ canvasElement }: { canvasElement: HTMLElement }) {
  const detector = 'Split anchor'
  const canvas = within(canvasElement)
  const lane = await canvas.findByTestId('transcript-scroll')
  await waitFor(() => expect(entry(lane, 'checkpoint/109'), `${detector}: latest turn`).toBeDefined())
  await waitFor(() => expect(gap(lane), `${detector}: initial following`).toBeLessThanOrEqual(1))
  // Read the leading turn with its folded work summary 8px above the lane top.
  await readerScroll(lane, 0)
  await waitFor(() => expect(summary(lane, 'lead/tool-a'), `${detector}: leading summary at the top`).toBeDefined())
  await readerScroll(lane, lane.scrollTop + offset(lane, summary(lane, 'lead/tool-a')!) + 8)
  const before = offset(lane, summary(lane, 'lead/tool-a')!)
  await expect(Math.abs(before + 8), `${detector}: reader line set up`).toBeLessThanOrEqual(1)
  const rowBefore = offset(lane, entry(lane, 'lead/tool-a')!)
  const topBefore = lane.scrollTop

  await canvas.getByRole('button', { name: 'Deliver earlier prompt' }).click()
  await waitFor(() => expect(summary(lane, 'lead/prompt'), `${detector}: renamed summary`).toBeDefined())
  await settle()
  const leaf = summary(lane, 'lead/prompt')!
  const row = entry(lane, 'lead/prompt')!
  await expect(Math.abs(offset(lane, leaf) - before), `${detector}: reader line restored`).toBeLessThanOrEqual(1)
  // Controls: the saved entry id no longer exists, older content was inserted above the
  // reader, and the prompt moved the summary down inside its row, so neither the entry
  // id nor the saved row offset could have restored this line.
  await expect(entry(lane, 'lead/tool-a'), `${detector}: old entry retired`).toBeUndefined()
  await expect(lane.scrollTop - topBefore, `${detector}: content inserted above`).toBeGreaterThan(300)
  await expect(Math.abs(offset(lane, row) - rowBefore), `${detector}: row offset differs from the saved one`).toBeGreaterThan(20)
  await settle()
  await expect(Math.abs(offset(lane, summary(lane, 'lead/prompt')!) - before), `${detector}: reader line stays restored`).toBeLessThanOrEqual(1)
}

/** Text reflow without a React commit or DOM insertion, as when late font metrics or highlighting land. */
const growText = (host: HTMLElement) => {
  const walker = document.createTreeWalker(host, NodeFilter.SHOW_TEXT, { acceptNode: node => (node.textContent ?? '').trim().length > 20 ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP })
  const text = walker.nextNode() as Text
  text.data += ` ${prose('Late reflow', 8).replaceAll('\n\n', ' ')}`
}
const answers = (lane: HTMLElement) => [...lane.querySelectorAll<HTMLElement>('[data-item-id$="/answer"]')].filter(candidate => candidate.getBoundingClientRect().height > 0)

async function exerciseAsync({ canvasElement }: { canvasElement: HTMLElement }) {
  const detector = 'Async follow'
  const canvas = within(canvasElement)
  const lane = await canvas.findByTestId('transcript-scroll')
  await waitFor(() => expect(entry(lane, 'checkpoint/11'), `${detector}: last turn`).toBeDefined())
  await waitFor(() => expect(gap(lane), `${detector}: initial following`).toBeLessThanOrEqual(1))
  await settle()
  const last = entry(lane, 'checkpoint/11')!
  const height = last.getBoundingClientRect().height
  growText(answers(lane).find(candidate => candidate.dataset['itemId'] === 'checkpoint/11/answer')!)
  // Control: the row really grew before any observer delivered it.
  await expect(last.getBoundingClientRect().height - height, `${detector}: last row grew`).toBeGreaterThan(40)
  await waitFor(() => expect(gap(lane), `${detector}: pinned after growth`).toBeLessThanOrEqual(1))
  await settle()
  await expect(gap(lane), `${detector}: still pinned`).toBeLessThanOrEqual(1)

  // Control: a detached reader keeps its row through the same growth.
  await readerScroll(lane, lane.scrollTop - 600)
  const reading = [...lane.querySelectorAll<HTMLElement>('[data-embrace-entry-id]')].filter(candidate => candidate.getBoundingClientRect().height > 0 && offset(lane, candidate) <= 0).at(-1)!
  const line = offset(lane, reading)
  // Growth below the reader's row, inside the mounted window.
  growText(answers(lane).filter(candidate => offset(lane, candidate) > line && offset(lane, candidate) < lane.clientHeight).at(-1)!)
  await settle()
  await expect(Math.abs(offset(lane, reading) - line), `${detector}: reader row stays`).toBeLessThanOrEqual(1)
  await expect(gap(lane), `${detector}: reader not repinned`).toBeGreaterThan(300)
}

async function exerciseGesture({ canvasElement }: { canvasElement: HTMLElement }) {
  const detector = 'Upward gesture'
  for (const older of [false, true]) {
    const host = canvasElement.querySelector<HTMLElement>(`[data-gesture-lane="${older ? 'older' : 'complete'}"]`)!
    const region = within(host)
    const lane = await region.findByTestId('transcript-scroll')
    await waitFor(() => expect(entry(lane, 'checkpoint/200'), `${detector}: first turn`).toBeDefined())
    await settle()
    await expect(lane.scrollTop, `${detector}: nothing above`).toBe(0)
    const requests = Number(host.dataset['requests'])
    lane.dispatchEvent(new WheelEvent('wheel', { deltaY: -120, bubbles: true }))
    await settle()
    await region.getByRole('button', { name: /^Append replies/ }).click()
    await waitFor(() => expect(lane.scrollHeight - lane.clientHeight, `${detector}: replies overflow the lane`).toBeGreaterThan(300))
    await settle()
    if (!older) {
      // Complete history: the gesture moved nothing and read nothing, so following continues.
      await expect(lane.scrollTop, `${detector}: complete lane overflowed`).toBeGreaterThan(0)
      await expect(gap(lane), `${detector}: complete lane follows`).toBeLessThanOrEqual(1)
    } else {
      // Control: with older history the same gesture asks for it and reads from the top.
      await expect(Number(host.dataset['requests']), `${detector}: older page requested`).toBeGreaterThan(requests)
      await expect(lane.scrollTop, `${detector}: older lane reads from the top`).toBeLessThanOrEqual(1)
      await expect(gap(lane), `${detector}: older lane does not follow`).toBeGreaterThan(300)
    }
  }
}

export const SplitAnchorDark1440: Story = { play: exerciseSplit }
export const SplitAnchorLight1440: Story = { args: { scheme: 'light' }, play: exerciseSplit }
export const SplitAnchorDark440: Story = { args: { narrow: true }, play: exerciseSplit }
export const SplitAnchorLight440: Story = { args: { scheme: 'light', narrow: true }, play: exerciseSplit }
export const AsyncFollowDark1440: Story = { args: { scenario: 'async' }, play: exerciseAsync }
export const AsyncFollowLight1440: Story = { args: { scenario: 'async', scheme: 'light' }, play: exerciseAsync }
export const AsyncFollowDark440: Story = { args: { scenario: 'async', narrow: true }, play: exerciseAsync }
export const AsyncFollowLight440: Story = { args: { scenario: 'async', scheme: 'light', narrow: true }, play: exerciseAsync }
export const UpwardGestureDark1440: Story = { args: { scenario: 'gesture' }, play: exerciseGesture }
export const UpwardGestureLight1440: Story = { args: { scenario: 'gesture', scheme: 'light' }, play: exerciseGesture }
export const UpwardGestureDark440: Story = { args: { scenario: 'gesture', narrow: true }, play: exerciseGesture }
export const UpwardGestureLight440: Story = { args: { scenario: 'gesture', scheme: 'light', narrow: true }, play: exerciseGesture }
