import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Transcript, type TranscriptHandle, type TranscriptHistory, type TranscriptTurn } from './assistant-ui/composition/Transcript'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import type { ConversationItem, TextItem } from './assistant-ui/embrace-data/model'
import { workLogTurnFromItems } from './assistant-ui/taste/work-log'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const at = '2026-01-15T12:00:00Z'
const now = Date.parse(at)
const pageSize = 100
const initialStart = 300
const total = 2400
function makeTurn(index: number): TranscriptTurn {
  const id = `history/${index}`
  const prompt: TextItem & { role: 'user' } = { _tag: 'Text', id: `${id}/prompt`, role: 'user', attachments: [], streaming: false, at, text: `Review checkpoint ${index}: preserve the selected row while comparing the recorded output.` }
  const paragraphs = Array.from({ length: 1 + index % 7 }, (_, paragraph) => `Checkpoint ${index}, observation ${paragraph + 1}: The retained selection refers to the same resource. Compare the visible rows with the recorded input before changing the projection.`).join('\n\n')
  const items: readonly ConversationItem[] = [
    { _tag: 'ToolCall', id: `${id}/read`, callId: `${id}/read`, name: 'read', input: { path: `src/checkpoints/checkpoint-${index}.ts` }, status: 'success', callSeen: true, at, result: { content: `export const checkpoint = ${index}\nexport const selected = true\n`, isError: false, at } },
    { _tag: 'Reasoning', id: `${id}/thinking`, text: `Check the resource identity at checkpoint ${index} before updating the visible rows.`, streaming: false, at },
    { _tag: 'Text', id: `${id}/answer`, role: 'assistant', text: `${paragraphs}${index % 5 === 0 ? '\n\n```ts\nconst selected = rows.find(row => row.selected)\n```' : ''}`, attachments: [], streaming: false, at },
  ]
  return { id, prompt, items, work: workLogTurnFromItems(items, { kindFor: () => 'read', running: false, failed: false, interrupted: false, durationMs: 24000, startedAt: at, completeHistory: true }) }
}
// Shared immutable fixtures, matching Transcript.stories' runtime-source/work-log pattern.
const turns = Array.from({ length: total + 2 }, (_, index) => makeTurn(index))
const failures = ['none', 'dom-bound', 'prepend', 'prepend-focus', 'resize', 'theme', 'empty-page', 'unread', 'stale-jump', 'append-following', 'append-reading', 'fold', 'keyboard', 'retry', 'jump', 'start'] as const
type Failure = typeof failures[number]
type Page = { readonly _tag: 'Ready' } | { readonly _tag: 'Loading' } | { readonly _tag: 'Failed' }
const styles = stylex.create({
  root: { width: g.polishWide, maxWidth: '100%', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  narrow: { width: g.polishNarrow },
  controls: { display: 'flex', flexWrap: 'wrap', gap: s.md, paddingBlock: s.md },
  frame: { height: g.transcriptFrameHeight },
})
function PagedHistory({ scheme = 'dark', narrow = false, failure = 'none' }: { scheme?: Scheme; narrow?: boolean; failure?: Failure }) {
  const root = React.useRef<HTMLElement>(null)
  const handle = React.useRef<TranscriptHandle>(null)
  const [start, setStart] = React.useState(initialStart)
  const [end, setEnd] = React.useState(total)
  const [page, setPage] = React.useState<Page>({ _tag: 'Ready' })
  const [failNext, setFailNext] = React.useState(false)
  const [revision, setRevision] = React.useState(0)
  const [action, setAction] = React.useState('')
  const [requests, setRequests] = React.useState(0)
  const [resized, setResized] = React.useState(false)
  const [switchedTheme, setSwitchedTheme] = React.useState(false)
  const [followKey, setFollowKey] = React.useState<string | undefined>(undefined)
  const effectiveScheme = switchedTheme ? scheme === 'dark' ? 'light' : 'dark' : scheme
  const loadOlder = () => { setRequests(value => value + 1); setPage({ _tag: 'Loading' }); setAction('request'); setRevision(value => value + 1) }
  const deliver = () => {
    if (failNext) { setFailNext(false); setPage({ _tag: 'Failed' }); setAction('error') }
    else { setStart(value => Math.max(0, value - pageSize)); setPage({ _tag: 'Ready' }); setAction('prepend') }
    setRevision(value => value + 1)
  }
  const history: TranscriptHistory = start === 0 ? { _tag: 'Complete' } : page._tag === 'Loading' ? { _tag: 'Loading' } : page._tag === 'Failed' ? { _tag: 'Failed', retry: failure === 'retry' ? () => {} : loadOlder } : { _tag: 'More', loadOlder }
  const loaded = React.useMemo(() => turns.slice(start, end), [start, end])
  const messages = React.useMemo(() => loaded.flatMap(turn => [turn.prompt!, ...turn.items]), [loaded])
  // Like LiveCritiqueR2: selected negative controls change the real rendered tree or
  // host action, so the same geometry/behavior assertion fails, not a fabricated flag.
  React.useLayoutEffect(() => {
    const lane = root.current?.querySelector<HTMLElement>('[data-testid="transcript-scroll"]')
    if (lane === null || lane === undefined) return
    const frame = requestAnimationFrame(() => {
      if (failure === 'prepend' && action === 'prepend') lane.scrollTop = 0
      if (failure === 'prepend-focus' && action === 'prepend') lane.focus()
      if (failure === 'resize' && action === 'resize') lane.scrollTop = 0
      if (failure === 'theme' && action === 'theme') lane.scrollTop = 0
      if (failure === 'empty-page' && action === 'empty') loadOlder()
      if (failure === 'unread' && action === 'append') {
        const unread = [...root.current!.querySelectorAll('button')].find(button => button.textContent?.includes('New messages'))
        if (unread !== undefined) unread.hidden = true
      }
      if (failure === 'stale-jump' && action === 'prepend' && followKey !== undefined) handle.current?.jumpTo('history/50')
      if (failure === 'append-reading' && action === 'append') lane.scrollTop = lane.scrollHeight
      if (failure === 'append-following' && action === 'append') lane.scrollTop = 0
      if (failure === 'start' && start === 0) lane.querySelector('[data-testid="history-start"]')?.remove()
      if (failure === 'dom-bound' && revision > 0) {
        const row = lane.querySelector('[data-testid="transcript-turn"]')
        if (row !== null) for (let index = 0; index < 250; index++) lane.append(row.cloneNode(true))
      }
    })
    return () => cancelAnimationFrame(frame)
  }, [failure, revision, action, start])
  return <main ref={root} data-scheme={effectiveScheme} data-requests={requests} data-loaded={loaded.length} data-start={start} data-page={page._tag} data-end={end} {...stylex.props(styles.root, (narrow !== resized) && styles.narrow, ...baselineTheme, effectiveScheme === 'light' && lightTheme)}>
    <div {...stylex.props(styles.controls)}>
      <button type="button" onClick={() => { setAction('control'); setRevision(value => value + 1) }}>Apply selected regression</button>
      <button type="button" disabled={page._tag !== 'Ready' || start === 0} onClick={loadOlder}>Request older page</button>
      <button type="button" disabled={page._tag !== 'Loading'} onMouseDown={event => event.preventDefault()} onClick={deliver}>Deliver older page</button>
      <button type="button" disabled={page._tag !== 'Loading'} onClick={() => { setPage({ _tag: 'Ready' }); setAction('empty'); setRevision(value => value + 1) }}>Deliver empty page</button>
      <button type="button" onMouseDown={event => event.preventDefault()} onClick={() => { setResized(value => !value); setAction('resize'); setRevision(value => value + 1) }}>Resize reading lane</button>
      <button type="button" onMouseDown={event => event.preventDefault()} onClick={() => { setSwitchedTheme(value => !value); setAction('theme'); setRevision(value => value + 1) }}>Switch reading theme</button>
      <button type="button" onClick={() => setFollowKey(value => value === undefined ? 'latest/1' : `${value}/next`)}>Follow latest</button>
      <button type="button" onClick={() => setFailNext(true)}>Fail next page</button>
      <button type="button" disabled={end === total + 2} onMouseDown={event => event.preventDefault()} onClick={() => { setEnd(value => value + 1); setAction('append'); setRevision(value => value + 1) }}>Append turn</button>
      <button type="button" onClick={() => { if (failure !== 'jump') handle.current?.jumpTo('history/50') }}>Jump to unloaded checkpoint 50</button>
      <button type="button" onClick={() => handle.current?.jumpTo('history/1300')}>Jump to loaded checkpoint 1300</button>
    </div>
    <section aria-label="Paged conversation" {...stylex.props(styles.frame)} onKeyDownCapture={event => { if (failure === 'keyboard' && (event.key === 'Home' || event.key === 'PageUp')) { event.preventDefault(); event.stopPropagation() } }} onClickCapture={event => { if (failure === 'fold' && (event.target as Element).closest('[data-testid="work-log"]')) { event.preventDefault(); event.stopPropagation() } }}>
      <EmbraceRuntimeProvider options={{ messages, onNew: async () => {} }}><Transcript ref={handle} scrollToBottomKey={followKey} title="Checkpoint review" turns={loaded} history={history} sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} onOpenTool={() => {}} /></EmbraceRuntimeProvider>
    </section>
  </main>
}
const meta = { title: 'Fractal UI/Transcript virtual history', component: PagedHistory, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark', narrow: false, failure: 'none' }, argTypes: { scheme: { control: 'radio', options: ['light', 'dark'] }, narrow: { control: 'boolean' }, failure: { control: 'select', options: failures, description: 'Restore one broken behavior. The corresponding production Transcript play assertion must fail.' } } } satisfies Meta<typeof PagedHistory>
export default meta
type Story = StoryObj<typeof meta>

async function exerciseHistory({ canvasElement }: { canvasElement: HTMLElement }) {
  const canvas = within(canvasElement)
  const root = canvasElement.querySelector('main')!
  const lane = await canvas.findByTestId('transcript-scroll')
  const row = (id: number) => lane.querySelector<HTMLElement>(`[data-embrace-entry-id="history/${id}"]`)
  const bottomGap = () => lane.scrollHeight - lane.scrollTop - lane.clientHeight
  const scrollTo = (top: number) => { lane.scrollTop = top; lane.dispatchEvent(new Event('scroll')) }
  const mountedBound = async () => { await expect(Number(root.getAttribute('data-loaded'))).toBeGreaterThanOrEqual(2100); await expect(lane.querySelectorAll('[data-testid="transcript-turn"]').length).toBeLessThan(120) }
  const deliver = async (start: number) => {
    await waitFor(() => expect(root).toHaveAttribute('data-page', 'Loading'))
    await userEvent.click(canvas.getByRole('button', { name: 'Deliver older page' }))
    await waitFor(() => expect(root).toHaveAttribute('data-start', String(start)))
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  }
  await waitFor(() => expect(lane.querySelectorAll('[data-testid="transcript-turn"]').length).toBeGreaterThan(0))
  await userEvent.click(canvas.getByRole('button', { name: 'Apply selected regression' }))
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  await mountedBound()
  await expect(canvas.queryByTestId('history-start')).toBeNull()

  // Following appends stay at the real bottom; reading appends keep a pixel anchor.
  scrollTo(lane.scrollHeight)
  await waitFor(() => expect(bottomGap()).toBeLessThanOrEqual(1))
  await userEvent.click(canvas.getByRole('button', { name: 'Append turn' }))
  await waitFor(() => expect(row(2400)).not.toBeNull())
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  await waitFor(() => expect(bottomGap()).toBeLessThanOrEqual(1))
  await userEvent.click(canvas.getByRole('button', { name: 'Jump to loaded checkpoint 1300' }))
  await waitFor(() => expect(row(1300)).not.toBeNull())
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  const anchor = row(1300)!
  const readingTop = anchor.getBoundingClientRect().top
  const heldReader = within(anchor).getByRole('button', { name: /Worked/ })
  heldReader.focus({ preventScroll: true })
  await userEvent.click(canvas.getByRole('button', { name: 'Append turn' }))
  await waitFor(() => expect(root).toHaveAttribute('data-end', '2402'))
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  await expect(anchor.getBoundingClientRect().top - readingTop).toBe(0)
  await expect(bottomGap()).toBeGreaterThan(lane.clientHeight)
  await expect(document.activeElement).toBe(heldReader)
  await expect(canvas.getByRole('button', { name: /New messages/ })).toBeVisible()

  // Remeasure wrapping and theme without moving the current reading anchor.
  for (const name of ['Resize reading lane', 'Switch reading theme', 'Resize reading lane', 'Switch reading theme']) {
    const top = row(1300)!.getBoundingClientRect().top
    await userEvent.click(canvas.getByRole('button', { name }))
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
    await expect(row(1300)!.getBoundingClientRect().top - top).toBe(0)
  }

  // A manually completed asynchronous prepend must move the reading anchor by 0px.
  await userEvent.click(canvas.getByRole('button', { name: 'Request older page' }))
  const focusedBeforePrepend = within(row(1300)!).getByRole('button', { name: /Worked/ })
  focusedBeforePrepend.focus()
  await deliver(200)
  await waitFor(() => expect(row(1300)?.getBoundingClientRect().top - readingTop).toBe(0))
  await expect(document.activeElement).toBe(focusedBeforePrepend)
  await mountedBound()

  // Expand and fold a measured mid-list work log, retaining its turn and reading position.
  const work = within(row(1300)!).getByTestId('work-log')
  const fold = within(work).getByRole('button', { name: /Worked/ })
  const foldTop = row(1300)!.getBoundingClientRect().top
  await userEvent.click(fold)
  await expect(fold).toHaveAttribute('aria-expanded', 'true')
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  const expandedHeight = row(1300)!.getBoundingClientRect().height
  await userEvent.click(fold)
  await expect(fold).toHaveAttribute('aria-expanded', 'false')
  await waitFor(() => expect(row(1300)!.getBoundingClientRect().height).toBeLessThan(expandedHeight))
  await expect(row(1300)!.getBoundingClientRect().top - foldTop).toBe(0)
  await mountedBound()

  // Native keyboard gestures page the scroll lane and request history at its top.
  lane.focus()
  const beforePageUp = lane.scrollTop
  await userEvent.keyboard('{PageUp}')
  await waitFor(() => expect(lane.scrollTop).toBeLessThan(beforePageUp))
  // Empty success retains the oldest ID and must not trigger a request loop.
  lane.focus()
  await userEvent.keyboard('{Home}')
  await waitFor(() => expect(root).toHaveAttribute('data-page', 'Loading'))
  const requestsBeforeEmpty = root.getAttribute('data-requests')
  await userEvent.click(canvas.getByRole('button', { name: 'Deliver empty page' }))
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))))
  await expect(root).toHaveAttribute('data-page', 'Ready')
  await expect(root).toHaveAttribute('data-start', '200')
  await expect(root).toHaveAttribute('data-requests', requestsBeforeEmpty!)
  await userEvent.click(canvas.getByRole('button', { name: 'Fail next page' }))
  await userEvent.click(canvas.getByRole('button', { name: 'Request older page' }))
  await userEvent.click(canvas.getByRole('button', { name: 'Deliver older page' }))
  await waitFor(() => expect(root).toHaveAttribute('data-page', 'Failed'))
  await expect(await canvas.findByTestId('history-error')).toHaveTextContent('Earlier messages could not be loaded')
  await expect(root).toHaveAttribute('data-start', '200')
  await userEvent.click(canvas.getByTestId('history-retry'))
  await deliver(100)
  await expect(canvas.queryByTestId('history-error')).toBeNull()

  // The imperative jump holds an unloaded target across page adoption, then lands on it.
  await userEvent.click(canvas.getByRole('button', { name: 'Jump to unloaded checkpoint 50' }))
  await deliver(0)
  await waitFor(() => expect(row(50)).not.toBeNull())
  await waitFor(() => {
    const target = row(50)!.getBoundingClientRect(), viewport = lane.getBoundingClientRect()
    expect(target.top).toBeGreaterThanOrEqual(viewport.top)
    expect(target.top).toBeLessThan(viewport.bottom)
  })
  await expect(canvas.queryByTestId('history-start')).toBeNull()
  lane.focus()
  await userEvent.keyboard('{Home}')
  await waitFor(() => expect(lane.scrollTop).toBe(0))
  await expect(await canvas.findByTestId('history-start')).toHaveTextContent('Start of conversation')
  await expect(canvas.queryByTestId('history-loading')).toBeNull()
  await mountedBound()
}
export const Dark1440: Story = { play: exerciseHistory }
export const Light1440: Story = { args: { scheme: 'light' }, play: exerciseHistory }
export const Dark440: Story = { args: { narrow: true }, play: exerciseHistory }
export const Light440: Story = { args: { scheme: 'light', narrow: true }, play: exerciseHistory }

async function exerciseNewerFollow({ canvasElement }: { canvasElement: HTMLElement }) {
  const canvas = within(canvasElement)
  const root = canvasElement.querySelector('main')!
  const lane = await canvas.findByTestId('transcript-scroll')
  await waitFor(() => expect(lane.querySelector('[data-testid="transcript-turn"]')).not.toBeNull())
  await userEvent.click(canvas.getByRole('button', { name: 'Jump to unloaded checkpoint 50' }))
  await waitFor(() => expect(root).toHaveAttribute('data-page', 'Loading'))
  await userEvent.click(canvas.getByRole('button', { name: 'Follow latest' }))
  await waitFor(() => expect(lane.scrollHeight - lane.scrollTop - lane.clientHeight).toBeLessThanOrEqual(1))
  for (const start of [200, 100, 0]) {
    if (start !== 200) await userEvent.click(canvas.getByRole('button', { name: 'Request older page' }))
    await userEvent.click(canvas.getByRole('button', { name: 'Deliver older page' }))
    await waitFor(() => expect(root).toHaveAttribute('data-start', String(start)))
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
    await expect(lane.scrollHeight - lane.scrollTop - lane.clientHeight).toBeLessThanOrEqual(1)
  }
  await expect(lane.querySelector('[data-embrace-entry-id="history/50"]')).toBeNull()
}
export const NewerFollowDark1440: Story = { play: exerciseNewerFollow }
export const NewerFollowLight1440: Story = { args: { scheme: 'light' }, play: exerciseNewerFollow }
export const NewerFollowDark440: Story = { args: { narrow: true }, play: exerciseNewerFollow }
export const NewerFollowLight440: Story = { args: { scheme: 'light', narrow: true }, play: exerciseNewerFollow }
function HistoryStates({ scheme = 'dark', narrow = false }: { scheme?: Scheme; narrow?: boolean }) {
  const [requested, setRequested] = React.useState(false)
  const [retried, setRetried] = React.useState(false)
  const examples: readonly { readonly label: string; readonly history: TranscriptHistory }[] = [
    { label: 'Complete', history: { _tag: 'Complete' } },
    { label: 'More', history: { _tag: 'More', loadOlder: () => setRequested(true) } },
    { label: 'Loading', history: { _tag: 'Loading' } },
    { label: 'Failed', history: { _tag: 'Failed', retry: () => setRetried(true) } },
  ]
  return <main data-scheme={scheme} {...stylex.props(styles.root, narrow && styles.narrow, ...baselineTheme, scheme === 'light' && lightTheme)}>
    {examples.map(({ label, history }) => <section key={label} aria-label={`${label} history example`}>
      <h2>{label}</h2>
      <div {...stylex.props(styles.frame)}><EmbraceRuntimeProvider options={{ messages: [turns[0]!.prompt!, ...turns[0]!.items], onNew: async () => {} }}><Transcript title={`${label} checkpoint review`} landmarkContext={label} turns={[turns[0]!]} history={history} sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} /></EmbraceRuntimeProvider></div>
    </section>)}
    <output aria-label="History actions">{requested ? 'Older page requested. ' : ''}{retried ? 'Failed page retried.' : ''}</output>
  </main>
}
export const AllStates: Story = { render: args => <HistoryStates scheme={args.scheme} narrow={args.narrow} />, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const complete = within(canvas.getByRole('region', { name: 'Complete history example' }))
  await expect(await complete.findByTestId('history-start')).toHaveTextContent('Start of conversation')
  const loading = within(canvas.getByRole('region', { name: 'Loading history example' }))
  await expect(await loading.findByTestId('history-loading')).toHaveTextContent('Loading earlier messages')
  const failed = within(canvas.getByRole('region', { name: 'Failed history example' }))
  await expect(await failed.findByTestId('history-error')).toHaveTextContent('Earlier messages could not be loaded')
  await userEvent.click(failed.getByTestId('history-retry'))
  await expect(canvas.getByLabelText('History actions')).toHaveTextContent('Failed page retried.')
  const more = within(canvas.getByRole('region', { name: 'More history example' })).getByTestId('transcript-scroll')
  more.focus()
  await userEvent.keyboard('{Home}')
  await waitFor(() => expect(canvas.getByLabelText('History actions')).toHaveTextContent('Older page requested.'))
} }
export const AllStatesLight: Story = { ...AllStates, args: { scheme: 'light' } }
