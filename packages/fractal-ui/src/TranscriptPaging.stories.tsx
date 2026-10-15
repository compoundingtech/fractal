import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { renderToString } from 'react-dom/server'
import { hydrateRoot } from 'react-dom/client'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Transcript, type TranscriptHistory, type TranscriptTurn } from './assistant-ui/composition/Transcript'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import type { TextItem } from './assistant-ui/embrace-data/model'
import { workLogTurnFromItems } from './assistant-ui/taste/work-log'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { surfaceVars, textVars, typeVars } from './assistant-ui/composition-tokens.stylex'

const at = '2026-01-15T12:00:00Z'
const now = Date.parse(at)
const turns: readonly TranscriptTurn[] = Array.from({ length: 2100 }, (_, index) => {
  const prompt: TextItem & { role: 'user' } = { _tag: 'Text', id: `paging/${index}/prompt`, role: 'user', text: `Review checkpoint ${index} and preserve the selected resource.`, attachments: [], streaming: false, at }
  const answer: TextItem = { _tag: 'Text', id: `paging/${index}/answer`, role: 'assistant', text: `Checkpoint ${index}: the selected resource still matches the recorded input.\n\n${'Compare the visible output before changing the projection. '.repeat(1 + index % 5)}`, attachments: [], streaming: false, at }
  return { id: `paging/${index}`, prompt, items: [answer], work: workLogTurnFromItems([answer], { kindFor: () => 'read', running: false, failed: false, interrupted: false, completeHistory: true }) }
})
const pending: TextItem & { role: 'user' } = { _tag: 'Text', id: 'paging/pending', role: 'user', text: 'Please verify the next checkpoint.', attachments: [], streaming: false, at, sendState: { _tag: 'Unconfirmed', detail: 'Sending it again is safe.' } }
const pendingTurn: TranscriptTurn = { id: pending.id, prompt: pending, items: [], work: workLogTurnFromItems([], { kindFor: () => 'read', running: false, failed: false, interrupted: false, completeHistory: true }) }
const styles = stylex.create({
  root: { backgroundColor: surfaceVars.canvas, color: textVars.fg, fontFamily: typeVars.fontSans, padding: 16, minWidth: 0 },
  frame: { height: 650, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' },
  states: { display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(min(100%, 360px), 1fr))', gap: 16 },
})
type PagingState = 'more' | 'complete' | 'loading' | 'error'
function RuntimeTranscript({ loaded, history }: { loaded: readonly TranscriptTurn[]; history: TranscriptHistory }) {
  const messages = React.useMemo(() => loaded.flatMap(turn => turn.prompt === undefined ? turn.items : [turn.prompt, ...turn.items]), [loaded])
  return <EmbraceRuntimeProvider options={{ messages, onNew: async () => {} }}><Transcript title="Checkpoint review" turns={loaded} history={history} sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} onRetrySend={() => {}} /></EmbraceRuntimeProvider>
}
function Paging({ state = 'more', scheme = 'dark', embedded = false }: { state?: PagingState; scheme?: Scheme; embedded?: boolean }) {
  const Root = embedded ? 'div' : 'main'
  const [start, setStart] = React.useState(state === 'complete' ? 0 : 100)
  const [page, setPage] = React.useState<PagingState>(state)
  const [requests, setRequests] = React.useState(0)
  const [failNext, setFailNext] = React.useState(false)
  const finish = React.useRef<(() => void) | undefined>(undefined)
  const loadOlder = () => new Promise<void>(resolve => {
    setRequests(value => value + 1)
    setPage('loading')
    finish.current = () => { setPage(failNext ? 'error' : 'complete'); if (!failNext) setStart(0); setFailNext(false); resolve() }
  })
  const history: TranscriptHistory = page === 'complete' ? { _tag: 'Complete' } : page === 'loading' ? { _tag: 'Loading' } : page === 'error' ? { _tag: 'Failed', retry: loadOlder, detail: 'The connection dropped. Loaded checkpoints are retained.' } : { _tag: 'More', loadOlder }
  const loaded = React.useMemo(() => turns.slice(start), [start])
  return <Root data-scheme={scheme} data-page={page} data-requests={requests} data-loaded={loaded.length} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <button type="button" disabled={page !== 'loading'} onMouseDown={event => event.preventDefault()} onClick={() => finish.current?.()}>Deliver older page</button>
    <button type="button" onClick={() => setFailNext(true)}>Fail next page</button>
    <section aria-label={`Paged checkpoint review, ${state}`} {...stylex.props(styles.frame)}><RuntimeTranscript loaded={loaded} history={history} /></section>
  </Root>
}
function Hydration({ scheme = 'dark' }: { scheme?: Scheme }) {
  const host = React.useRef<HTMLDivElement>(null)
  const element = <RuntimeTranscript loaded={[pendingTurn]} history={{ _tag: 'Complete' }} />
  const [markup] = React.useState(() => renderToString(element))
  React.useLayoutEffect(() => {
    const node = host.current!
    const root = hydrateRoot(node, element, { onRecoverableError: error => { node.dataset.hydrationError = String(error) } })
    return () => root.unmount()
  }, [])
  return <main data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}><section ref={host} data-testid="hydration-host" {...stylex.props(styles.frame)} dangerouslySetInnerHTML={{ __html: markup }} /></main>
}
const meta = { title: 'Fractal UI/Transcript paging', component: Paging, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark', state: 'more' }, argTypes: { scheme: { control: 'radio', options: ['dark', 'light'] }, state: { control: 'select', options: ['more', 'complete', 'loading', 'error'] } } } satisfies Meta<typeof Paging>
export default meta
type Story = StoryObj<typeof meta>
const nextFrame = () => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
export const LongThread: Story = { play: async ({ canvasElement }) => {
  const lane = await waitFor(() => { const value = canvasElement.querySelector<HTMLElement>('[data-testid="transcript-scroll"]'); expect(value).not.toBeNull(); return value! })
  await nextFrame()
  await waitFor(() => expect(lane.scrollTop).toBeGreaterThan(1000))
  expect(canvasElement.querySelectorAll('[data-testid="transcript-turn"]').length).toBeLessThan(100)
  // Reader intent is required: programmatic scrolling alone must not fetch pages.
  lane.dispatchEvent(new WheelEvent('wheel', { deltaY: -10000, bubbles: true }))
  lane.scrollTop = 0
  lane.dispatchEvent(new Event('scroll', { bubbles: true }))
  const host = canvasElement.querySelector('[data-requests]')!
  await waitFor(() => expect(host).toHaveAttribute('data-page', 'loading'))
  await nextFrame()
  const anchor = await waitFor(() => {
    const viewport = lane.getBoundingClientRect()
    const row = [...lane.querySelectorAll<HTMLElement>('[data-testid="transcript-turn"]')].find(row => row.getBoundingClientRect().bottom > viewport.top && row.getBoundingClientRect().top < viewport.bottom)
    expect(row).toBeDefined()
    return row!
  })
  await nextFrame()
  const id = anchor!.getAttribute('data-item-id')!
  const offset = anchor!.getBoundingClientRect().top - lane.getBoundingClientRect().top
  await userEvent.click(within(canvasElement).getByRole('button', { name: 'Deliver older page' }))
  await waitFor(() => expect(host).toHaveAttribute('data-loaded', '2100'))
  await nextFrame()
  await waitFor(() => {
    const retained = [...lane.querySelectorAll<HTMLElement>('[data-testid="transcript-turn"]')].find(row => row.getAttribute('data-item-id') === id)
    expect(retained).toBeDefined()
    expect(Math.abs(retained!.getBoundingClientRect().top - lane.getBoundingClientRect().top - offset)).toBeLessThan(4)
  })
  expect(host).toHaveAttribute('data-requests', '1')
} }
export const FirstPageLoaded: Story = { args: { state: 'complete' }, play: async ({ canvasElement }) => {
  const lane = await waitFor(() => { const value = canvasElement.querySelector<HTMLElement>('[data-testid="transcript-scroll"]'); expect(value).not.toBeNull(); return value! })
  await waitFor(() => expect(lane.scrollTop).toBeGreaterThan(1000))
  lane.dispatchEvent(new WheelEvent('wheel', { deltaY: -10000, bubbles: true }))
  lane.scrollTop = 0
  lane.dispatchEvent(new Event('scroll', { bubbles: true }))
  await waitFor(() => expect(canvasElement.querySelector('[data-testid="history-boundary"]')).toHaveTextContent('Start of conversation'))
} }
export const LoadingOlder: Story = { args: { state: 'loading' }, play: async ({ canvasElement }) => {
  await waitFor(() => expect(canvasElement.querySelector('[data-page]')).toHaveAttribute('data-page', 'loading'))
} }
export const LoadError: Story = { args: { state: 'error' }, play: async ({ canvasElement }) => {
  const lane = await waitFor(() => { const value = canvasElement.querySelector<HTMLElement>('[data-testid="transcript-scroll"]'); expect(value).not.toBeNull(); return value! })
  await waitFor(() => expect(lane.scrollTop).toBeGreaterThan(1000))
  lane.dispatchEvent(new WheelEvent('wheel', { deltaY: -10000, bubbles: true }))
  lane.scrollTop = 0
  lane.dispatchEvent(new Event('scroll', { bubbles: true }))
  const retry = await within(canvasElement).findByTestId('history-retry')
  await waitFor(() => expect(getComputedStyle(lane.firstElementChild!).pointerEvents).not.toBe('none'))
  await userEvent.click(retry)
  await waitFor(() => expect(canvasElement.querySelector('[data-page]')).toHaveAttribute('data-page', 'loading'))
} }
export const PendingHydration: Story = { render: args => <Hydration scheme={args.scheme} />, play: async ({ canvasElement }) => {
  const host = within(canvasElement).getByTestId('hydration-host')
  await nextFrame()
  const assertSingle = async () => { await expect(host.querySelectorAll('[data-testid="send-unconfirmed"]')).toHaveLength(1) }
  await waitFor(assertSingle)
  expect(host).not.toHaveAttribute('data-hydration-error')
  // Negative control lives in the harness: the real assertion must reject a duplicated row.
  const duplicate = host.querySelector('[data-testid="send-unconfirmed"]')!.cloneNode(true)
  host.append(duplicate)
  try { await expect(assertSingle()).rejects.toThrow() } finally { host.removeChild(duplicate) }
  await assertSingle()
} }
export const AllStates: Story = { render: args => <main {...stylex.props(styles.states)}>{(['complete', 'more', 'loading', 'error'] as const).map(state => <section key={state} aria-label={`${state} paging state`}><h2>{state}</h2><Paging state={state} scheme={args.scheme} embedded /></section>)}</main> }
