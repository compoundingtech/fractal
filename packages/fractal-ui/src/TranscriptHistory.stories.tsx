// Keyboard plays focus an active RAC row and exercise its synthetic React key
// handling unchanged. Native scroller-focus defaults belong to the design
// browser gate; this fixture has no native-input bridge or keyboard scroll shim.
// Headless Chromium exposes overlay scrollbars (clientWidth === offsetWidth),
// so these plays cannot target a native thumb gutter and claim no drag coverage.
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
const failures = ['none', 'dom-bound', 'prepend', 'prepend-focus', 'resize', 'theme', 'empty-page', 'empty-latch', 'touch-latch', 'touch-scroll-required', 'failed-auto-retry', 'unread', 'stale-jump', 'append-following', 'append-reading', 'fold', 'keyboard', 'retry', 'jump', 'start', 'pending-default', 'pending-explicit', 'jump-anchored', 'jump-missing', 'jump-superseded'] as const
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
  const pendingPage = React.useRef<{ promise: Promise<void>; resolve: () => void; reject: (error: Error) => void } | null>(null)
  const queuedJump = React.useRef(false)
  const [jumpResult, setJumpResult] = React.useState('')
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
  const loadOlder = (): Promise<void> => {
    if (pendingPage.current !== null) return pendingPage.current.promise
    if (failure === 'empty-latch' && action === 'empty') return Promise.resolve()
    let resolve!: () => void
    let reject!: (error: Error) => void
    const promise = new Promise<void>((accept, fail) => { resolve = accept; reject = fail })
    pendingPage.current = { promise, resolve, reject }
    setRequests(value => value + 1)
    setPage({ _tag: 'Loading' })
    setAction('request')
    setRevision(value => value + 1)
    return promise
  }
  const requestOlder = () => { void loadOlder().catch(error => console.warn('History fixture request failed', error)) }
  const finishPage = (error?: Error) => {
    const pending = pendingPage.current
    pendingPage.current = null
    if (error === undefined) pending?.resolve()
    else pending?.reject(error)
  }
  const deliver = () => {
    if (failNext) {
      setFailNext(false); setPage({ _tag: 'Failed' }); setAction('error')
      finishPage(new Error('Checkpoint source unavailable'))
    } else {
      setStart(value => Math.max(0, value - pageSize)); setPage({ _tag: 'Ready' }); setAction('prepend')
      finishPage()
    }
    setRevision(value => value + 1)
  }
  const history: TranscriptHistory = start === 0 ? failure === 'start' ? { _tag: 'More', loadOlder: async () => {} } : { _tag: 'Complete' } : page._tag === 'Loading' ? { _tag: 'Loading' } : page._tag === 'Failed' ? { _tag: 'Failed', retry: failure === 'retry' ? async () => {} : loadOlder, detail: 'Checkpoint source unavailable' } : { _tag: 'More', loadOlder }
  // The host loads an absent target first. Only a committed loaded snapshot may
  // issue jumpTo: an absent target now resolves Missing on the next commit.
  React.useLayoutEffect(() => {
    if (!queuedJump.current || page._tag !== 'Ready') return
    if (start > 50) { requestOlder(); return }
    queuedJump.current = false
    const promise = handle.current?.jumpTo('history/50/answer')
    if (promise !== undefined) void promise.then(result => setJumpResult(result._tag))
  }, [start, page._tag])
  const loaded = React.useMemo(() => turns.slice(start, end), [start, end])
  const messages = React.useMemo(() => loaded.flatMap(turn => [turn.prompt!, ...turn.items]), [loaded])
  // Like LiveCritiqueR2: selected negative controls change the real rendered tree or
  // host action, so the same geometry/behavior assertion fails, not a fabricated flag.
  React.useLayoutEffect(() => {
    const lane = root.current?.querySelector<HTMLElement>('[data-testid="transcript-scroll"]')
    if (lane === null || lane === undefined) return
    const geometryRegression = (failure === 'resize' && action === 'resize') || (failure === 'theme' && action === 'theme')
    const frame = requestAnimationFrame(() => {
      // Model losing reader mode during a geometry transition, rather than a
      // cosmetic transform that the production anchor correctly compensates.
      if (geometryRegression) setFollowKey(value => `${value ?? 'regression'}/lost-anchor`)
      if (failure === 'prepend-focus' && action === 'prepend') lane.focus()
      if (failure === 'empty-page' && action === 'empty') requestOlder()
      if (failure === 'unread' && action === 'append') {
        const unread = [...root.current!.querySelectorAll('button')].find(button => button.textContent?.includes('New messages'))
        if (unread !== undefined) unread.hidden = true
      }
      if (failure === 'stale-jump' && action === 'prepend' && followKey !== undefined) handle.current?.jumpTo('history/50/answer')
      if (failure === 'append-following' && action === 'append') handle.current?.jumpTo('history/1300/answer')
      if ((failure === 'prepend' && action === 'prepend') || (failure === 'append-reading' && action === 'append')) setFollowKey(value => `${value ?? 'regression'}/lost-anchor`)
      if (failure === 'dom-bound' && revision > 0) {
        const row = lane.querySelector('[data-testid="transcript-turn"]')
        if (row !== null) for (let index = 0; index < 250; index++) lane.append(row.cloneNode(true))
      }
    })
    return () => { cancelAnimationFrame(frame) }
  }, [failure, revision, action, start])
  return <main ref={root} data-scheme={effectiveScheme} data-jump-result={jumpResult} data-requests={requests} data-loaded={loaded.length} data-start={start} data-page={page._tag} data-end={end} {...stylex.props(styles.root, (narrow !== resized) && styles.narrow, ...baselineTheme, effectiveScheme === 'light' && lightTheme)}>
    <div {...stylex.props(styles.controls)}>
      <button type="button" onClick={() => { setAction('control'); setRevision(value => value + 1) }}>Apply selected regression</button>
      <button type="button" disabled={page._tag !== 'Ready' || start === 0} onClick={requestOlder}>Request older page</button>
      <button type="button" disabled={page._tag !== 'Loading'} onMouseDown={event => event.preventDefault()} onClick={deliver}>Deliver older page</button>
      <button type="button" disabled={page._tag !== 'Loading'} onClick={() => { setPage({ _tag: 'Ready' }); setAction('empty'); setRevision(value => value + 1); finishPage() }}>Deliver empty page</button>
      <button type="button" onMouseDown={event => event.preventDefault()} onClick={() => { setResized(value => !value); setAction('resize'); setRevision(value => value + 1) }}>Resize reading lane</button>
      <button type="button" onMouseDown={event => event.preventDefault()} onClick={() => { setSwitchedTheme(value => !value); setAction('theme'); setRevision(value => value + 1) }}>Switch reading theme</button>
      <button type="button" onClick={() => { queuedJump.current = false; setFollowKey(value => value === undefined ? 'latest/1' : `${value}/next`) }}>Follow latest</button>
      <button type="button" onClick={() => setFailNext(true)}>Fail next page</button>
      <button type="button" disabled={end === total + 2} onMouseDown={event => event.preventDefault()} onClick={() => { setEnd(value => value + 1); setAction('append'); setRevision(value => value + 1) }}>Append turn</button>
      <button type="button" onClick={() => { if (failure !== 'jump') { queuedJump.current = true; setJumpResult('Pending'); setRevision(value => value + 1); if (start > 50) requestOlder(); else { queuedJump.current = false; void handle.current?.jumpTo('history/50/answer').then(result => setJumpResult(result._tag)) } } }}>Jump to unloaded checkpoint 50</button>
      <button type="button" onClick={() => { queuedJump.current = false; void handle.current?.jumpTo('history/1300/answer').then(result => setJumpResult(result._tag)) }}>Jump to loaded checkpoint 1300</button>
      <button type="button" onClick={() => { queuedJump.current = false; void handle.current?.jumpTo('history/1302/answer').then(result => setJumpResult(result._tag)) }}>Jump to loaded checkpoint 1302</button>
    </div>
    <section aria-label="Paged conversation" {...stylex.props(styles.frame)} onScrollCapture={() => { if (failure === 'failed-auto-retry' && page._tag === 'Failed') requestOlder() }} onTouchStartCapture={event => { if (failure === 'touch-latch') event.stopPropagation() }} onTouchMoveCapture={event => { if (failure === 'touch-scroll-required' || failure === 'touch-latch') event.stopPropagation() }} onPointerDownCapture={event => { if (failure === 'touch-latch' && event.pointerType === 'touch') event.stopPropagation() }} onKeyDownCapture={event => {
      if (failure === 'keyboard' && (event.key === 'Home' || event.key === 'PageUp')) { event.preventDefault(); event.stopPropagation() }
    }} onClickCapture={event => { if (failure === 'fold' && (event.target as Element).closest('[data-testid="work-log"]')) { event.preventDefault(); event.stopPropagation() } }}>
      <EmbraceRuntimeProvider options={{ messages, onNew: async () => {} }}><Transcript ref={handle} scrollToBottomKey={followKey} title="Checkpoint review" turns={loaded} history={history} sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} onOpenTool={() => {}} /></EmbraceRuntimeProvider>
    </section>
  </main>
}
const meta = { title: 'Fractal UI/Transcript virtual history', component: PagedHistory, render: args => contractFailures.includes(args.failure as ContractFailure) ? <ContractFixture target={args.failure as ContractFailure} failure={args.failure} scheme={args.scheme} narrow={args.narrow} /> : <PagedHistory {...args} />, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark', narrow: false, failure: 'none' }, argTypes: { scheme: { control: 'radio', options: ['light', 'dark'] }, narrow: { control: 'boolean' }, failure: { control: 'select', options: failures, description: 'Restore one broken behavior. The corresponding production Transcript play assertion must fail.' } } } satisfies Meta<typeof PagedHistory>
export default meta
type Story = StoryObj<typeof meta>

function historyElementIsActionable(element: HTMLElement, lane: HTMLElement): boolean {
  return element.isConnected && lane.contains(element) && getComputedStyle(element).pointerEvents !== 'none' && element.getBoundingClientRect().height > 0
}

function activeHistoryRow(lane: HTMLElement, id: number): HTMLElement | null {
  const candidates = lane.querySelectorAll<HTMLElement>(`[data-embrace-entry-id="history/${id}"]`)
  for (const candidate of candidates) {
    if (historyElementIsActionable(candidate, lane)) return candidate
  }
  return null
}

async function focusHistoryKeyboardRow(lane: HTMLElement, id?: number): Promise<void> {
  const focusedRow = await waitFor(async () => {
    const viewport = lane.getBoundingClientRect()
    const candidates = id === undefined ? lane.querySelectorAll<HTMLElement>('[role="row"]') : [activeHistoryRow(lane, id)?.closest<HTMLElement>('[role="row"]') ?? null]
    let target: HTMLElement | null = null
    for (const candidate of candidates) {
      if (candidate === null || !historyElementIsActionable(candidate, lane)) continue
      const rectangle = candidate.getBoundingClientRect()
      if (rectangle.height > 0 && rectangle.bottom > viewport.top && rectangle.top < viewport.bottom) { target = candidate; break }
    }
    await expect(target !== null, 'History keyboard setup: active RAC row').toBe(true)
    if (target === null) return
    if (document.activeElement !== target) target.focus({ preventScroll: true })
    await expect(document.activeElement, 'History keyboard setup: RAC row focus identity').toBe(target)
    return target
  })
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
  await expect(document.activeElement === focusedRow, 'History keyboard setup: settled RAC row focus identity').toBe(true)
}

async function leaveHistoryBoundary(lane: HTMLElement): Promise<void> {
  await focusHistoryKeyboardRow(lane)
  // PageDown can legitimately retain a row taller than the viewport. RAC's
  // sequential row navigation exits the boundary regardless of that geometry.
  for (let index = 0; index < 12 && lane.scrollTop <= lane.clientHeight * 1.5; index++) {
    const previous = document.activeElement
    await userEvent.keyboard('{ArrowDown}')
    const id = await waitFor(() => {
      const focused = document.activeElement
      expect(focused !== previous && focused instanceof HTMLElement && lane.contains(focused)).toBe(true)
      return focused?.querySelector<HTMLElement>('[data-embrace-entry-id]')?.getAttribute('data-embrace-entry-id')
    })
    if (id !== null && id !== undefined) await focusHistoryKeyboardRow(lane, Number(id.slice('history/'.length)))
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  }
  await waitFor(() => expect(lane.scrollTop).toBeGreaterThan(lane.clientHeight * 1.5))
}
async function settleInitialHistoryPage(canvasElement: HTMLElement, lane: HTMLElement): Promise<number> {
  const canvas = within(canvasElement), root = canvasElement.querySelector('main')!
  // The first physical top observation can request a page before initial following
  // settles. Complete that real host request before starting a keyboard scenario.
  await new Promise<void>(resolve => setTimeout(resolve, 200))
  if (root.getAttribute('data-page') === 'Loading') {
    const expectedStart = Math.max(0, Number(root.getAttribute('data-start')) - pageSize)
    canvas.getByRole('button', { name: 'Deliver older page' }).click()
    await waitFor(() => expect(root.getAttribute('data-start')).toBe(String(expectedStart)))
    await waitFor(() => expect(root.getAttribute('data-page')).toBe('Ready'))
  }
  await waitFor(() => expect(lane.scrollHeight - lane.scrollTop - lane.clientHeight, 'History setup: initial following gap').toBeLessThanOrEqual(1), { timeout: 5000 })
  return Number(root.getAttribute('data-start'))
}



// Each detector has its own positive story and routes selected regressions directly
// to that detector. An unrelated assertion in the full tour cannot mask a control.
async function exerciseIsolated(canvasElement: HTMLElement, target: Failure | 'pointer-retry') {
  if (contractFailures.includes(target as ContractFailure)) return exerciseContract(canvasElement, target as ContractFailure)
  if (target === 'stale-jump') return exerciseNewerFollow({ canvasElement })
  if (target === 'touch-latch' || target === 'touch-scroll-required') return exerciseTouchAtZero({ canvasElement }, target)
  const detector = `History detector: ${target}`
  const canvas = within(canvasElement)
  const root = canvasElement.querySelector('main')!
  const lane = await canvas.findByTestId('transcript-scroll')
  const currentRow = (id: number) => activeHistoryRow(lane, id)
  await waitFor(() => expect(lane.querySelector('[data-testid="transcript-turn"]')).not.toBeNull())
  const settle = async () => {
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))))
    // Native Home/row transitions settle after the immediate key dispatch.
    await new Promise<void>(resolve => setTimeout(resolve, 200))
  }
  const page = async (start: number) => {
    await waitFor(() => expect(root).toHaveAttribute('data-page', 'Loading'))
    canvas.getByRole('button', { name: 'Deliver older page' }).click()
    await waitFor(() => expect(root).toHaveAttribute('data-start', String(start)))
    await settle()
  }
  if (target === 'dom-bound') {
    await userEvent.click(canvas.getByRole('button', { name: 'Apply selected regression' }))
    await settle()
    await expect(lane.querySelectorAll('[data-testid="transcript-turn"]').length, detector).toBeLessThan(120)
    return
  }
  if (target === 'jump' || target === 'start') {
    await userEvent.click(canvas.getByRole('button', { name: 'Jump to unloaded checkpoint 50' }))
    if (target === 'jump') await waitFor(() => expect(root.getAttribute('data-page'), detector).toBe('Loading'))
    for (const start of [200, 100, 0]) {
      if (start !== 200 && root.getAttribute('data-page') === 'Ready') canvas.getByRole('button', { name: 'Request older page' }).click()
      await page(start)
    }
    if (target === 'jump') {
      await waitFor(() => expect(currentRow(50), detector).not.toBeNull())
      await waitFor(() => expect(root.getAttribute('data-jump-result'), detector).toBe('Anchored'))
      const rect = currentRow(50)!.getBoundingClientRect(), viewport = lane.getBoundingClientRect()
      await expect(rect.top, detector).toBeGreaterThanOrEqual(viewport.top)
      await expect(rect.top, detector).toBeLessThan(viewport.bottom)
    } else {
      await focusHistoryKeyboardRow(lane)
      await userEvent.keyboard('{Home}')
      await waitFor(() => expect(canvas.queryByTestId('history-start') !== null, detector).toBe(true))
      await expect(canvas.getByTestId('history-start')).toHaveTextContent('Start of conversation')
    }
    return
  }
  if (target === 'empty-page' || target === 'empty-latch' || target === 'retry' || target === 'failed-auto-retry' || target === 'pointer-retry') {
    await settleInitialHistoryPage(canvasElement, lane)
    if (target === 'retry' || target === 'failed-auto-retry' || target === 'pointer-retry') await userEvent.click(canvas.getByRole('button', { name: 'Fail next page' }))
    await focusHistoryKeyboardRow(lane)
    await userEvent.keyboard('{Home}')
    await waitFor(() => expect(root).toHaveAttribute('data-page', 'Loading'))
    await waitFor(() => expect(lane.scrollTop).toBe(0))
    await expect(canvas.getByRole('grid')).toHaveAttribute('aria-busy', 'true')
    const requests = Number(root.getAttribute('data-requests'))
    if (target === 'empty-page' || target === 'empty-latch') {
      canvas.getByRole('button', { name: 'Deliver empty page' }).click()
      await settle()
      if (target === 'empty-page') {
        await expect(root.getAttribute('data-page'), detector).toBe('Ready')
        await expect(Number(root.getAttribute('data-requests')), detector).toBe(requests)
      } else {
        await leaveHistoryBoundary(lane)
        await userEvent.keyboard('{Home}')
        await waitFor(() => expect(root.getAttribute('data-page'), detector).toBe('Loading'))
        await expect(Number(root.getAttribute('data-requests')), detector).toBe(requests + 1)
      }
    } else {
      canvas.getByRole('button', { name: 'Deliver older page' }).click()
      await waitFor(() => expect(root).toHaveAttribute('data-page', 'Failed'))
      if (target === 'failed-auto-retry') {
        await focusHistoryKeyboardRow(lane)
        await userEvent.keyboard('{Home}')
        lane.dispatchEvent(new WheelEvent('wheel', { bubbles: true, deltaY: -120 }))
        lane.dispatchEvent(new Event('scroll'))
        await settle()
        await expect(root.getAttribute('data-page'), detector).toBe('Failed')
        await expect(Number(root.getAttribute('data-requests')), detector).toBe(requests)
      } else {
        await waitFor(() => expect([...lane.querySelectorAll<HTMLElement>('[data-testid="history-retry"]')].some(candidate => historyElementIsActionable(candidate, lane)), 'History setup: actionable Retry control').toBe(true))
        const retry = [...lane.querySelectorAll<HTMLElement>('[data-testid="history-retry"]')].find(candidate => historyElementIsActionable(candidate, lane))
        await expect(retry !== undefined, 'History setup: current Retry control').toBe(true)
        if (retry === undefined) return
        if (target === 'pointer-retry') await userEvent.click(retry)
        else {
          await focusHistoryKeyboardRow(lane)
          for (let index = 0; index < 60 && document.activeElement !== retry; index++) await userEvent.tab()
          await expect(retry).toHaveFocus()
          await userEvent.keyboard('{Enter}')
        }
        await waitFor(() => expect(root.getAttribute('data-page'), detector).toBe('Loading'))
        await expect(Number(root.getAttribute('data-requests')), detector).toBe(requests + 1)
      }
    }
    return
  }
  if (target === 'append-following') {
    lane.dispatchEvent(new WheelEvent('wheel', { bubbles: true, deltaY: 100000 }))
    lane.scrollTop = lane.scrollHeight
    lane.dispatchEvent(new Event('scroll'))
    await settle()
    await userEvent.click(canvas.getByRole('button', { name: 'Append turn' }))
    await waitFor(() => expect(root).toHaveAttribute('data-end', '2401'))
    await settle()
    await waitFor(() => expect(lane.scrollHeight - lane.scrollTop - lane.clientHeight, detector).toBeLessThanOrEqual(1))
    return
  }
  const readingId = target === 'keyboard' ? 1302 : 1300
  await userEvent.click(canvas.getByRole('button', { name: `Jump to loaded checkpoint ${readingId}` }))
  await waitFor(() => expect(root.getAttribute('data-jump-result'), 'History setup: checkpoint jump resolves Anchored').toBe('Anchored'), { timeout: 5000 })
  await waitFor(() => expect(currentRow(readingId)).not.toBeNull())
  await settle()
  if (target === 'keyboard') {
    await expect(currentRow(readingId)!.getBoundingClientRect().height, 'History keyboard setup: short row fits viewport').toBeLessThan(lane.clientHeight)
    await focusHistoryKeyboardRow(lane, readingId)
    const top = lane.scrollTop
    await userEvent.keyboard('{PageUp}')
    await waitFor(() => expect(lane.scrollTop, detector).toBeLessThan(top))
    return
  }
  await waitFor(() => expect(currentRow(1300) !== null, 'History setup: active semantic reading row').toBe(true))
  const baseline = currentRow(1300)?.getBoundingClientRect()
  await expect(baseline !== undefined, 'History setup: semantic row geometry').toBe(true)
  if (baseline === undefined) return
  const top = baseline.top - lane.getBoundingClientRect().top
  if (target === 'resize' || target === 'theme') {
    await userEvent.click(canvas.getByRole('button', { name: target === 'resize' ? 'Resize reading lane' : 'Switch reading theme' }))
    await settle()
    await waitFor(async () => {
      const rectangle = currentRow(1300)?.getBoundingClientRect()
      const offset = rectangle === undefined ? undefined : rectangle.top - lane.getBoundingClientRect().top - top
      await expect(offset !== undefined, detector).toBe(true)
      if (offset === undefined) return
      await expect(offset, detector).toBe(0)
    })
  } else if (target === 'prepend' || target === 'prepend-focus') {
    await userEvent.click(canvas.getByRole('button', { name: 'Request older page' }))
    const focused = within(currentRow(1300)!).getByRole('button', { name: /Worked/ })
    focused.focus({ preventScroll: true })
    await page(200)
    if (target === 'prepend') {
      await waitFor(() => expect(currentRow(1300), detector).not.toBeNull())
      await expect(currentRow(1300)!.getBoundingClientRect().top - lane.getBoundingClientRect().top - top, detector).toBe(0)
    } else await expect(document.activeElement, detector).toBe(focused)
  } else if (target === 'fold') {
    await waitFor(() => expect(getComputedStyle(within(currentRow(1300)!).getByRole('button', { name: /Worked/ })).pointerEvents).not.toBe('none'))
    await userEvent.click(within(currentRow(1300)!).getByRole('button', { name: /Worked/ }))
    await waitFor(() => expect(within(currentRow(1300)!).getByRole('button', { name: /Worked/ }).getAttribute('aria-expanded'), detector).toBe('true'))
    await waitFor(() => expect(currentRow(1300)!.getBoundingClientRect().top - lane.getBoundingClientRect().top - top, detector).toBe(0))
    const expanded = currentRow(1300)!.getBoundingClientRect().height
    await waitFor(() => expect(getComputedStyle(within(currentRow(1300)!).getByRole('button', { name: /Worked/ })).pointerEvents).not.toBe('none'))
    await userEvent.click(within(currentRow(1300)!).getByRole('button', { name: /Worked/ }))
    await settle()
    await waitFor(() => expect(within(currentRow(1300)!).getByRole('button', { name: /Worked/ }).getAttribute('aria-expanded'), detector).toBe('false'))
    await expect(currentRow(1300)!.getBoundingClientRect().height, detector).toBeLessThan(expanded)
  } else {
    const held = within(currentRow(1300)!).getByRole('button', { name: /Worked/ })
    held.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1, buttons: 1 }))
    try {
      canvas.getByRole('button', { name: 'Append turn' }).click()
      await waitFor(() => expect(root).toHaveAttribute('data-end', '2401'))
      await settle()
      if (target === 'unread') {
        const unread = canvas.queryByRole('button', { name: /New messages/ })
        await expect(unread !== null, detector).toBe(true)
        if (unread === null) return
        await expect(unread.hidden, detector).toBe(false)
        await expect(unread).toBeVisible()
      }
      else {
        await waitFor(() => expect(currentRow(1300), detector).not.toBeNull())
        await expect(currentRow(1300)!.getBoundingClientRect().top - lane.getBoundingClientRect().top - top, detector).toBe(0)
      }
    } finally {
      held.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerId: 1, buttons: 0 }))
    }
  }
}

export const DomBoundProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'dom-bound') }
export const PrependAnchorProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'prepend') }
export const PrependFocusProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'prepend-focus') }
export const ResizeAnchorProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'resize') }
export const ThemeAnchorProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'theme') }
export const EmptyPageProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'empty-page') }
export const EmptyReentryProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'empty-latch') }
export const FollowingAppendProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'append-following') }
export const ReadingAppendProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'append-reading') }
export const HeldUnreadProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'unread') }
export const FoldProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'fold') }
export const KeyboardProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'keyboard') }
export const KeyboardRetryProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'retry') }
export const PointerRetryProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'pointer-retry') }
export const FailedNoAutoRetryProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'failed-auto-retry') }
export const UnloadedJumpProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'jump') }
export const StartMarkerProbe: Story = { play: ({ canvasElement }) => exerciseIsolated(canvasElement, 'start') }

async function exerciseHistory({ canvasElement, args }: { canvasElement: HTMLElement; args: { failure?: Failure } }) {
  if (args.failure !== undefined && args.failure !== 'none') {
    await exerciseIsolated(canvasElement, args.failure)
    return
  }
  const canvas = within(canvasElement)
  const root = canvasElement.querySelector('main')!
  const lane = await canvas.findByTestId('transcript-scroll')
  const row = (id: number) => activeHistoryRow(lane, id)
  const bottomGap = () => lane.scrollHeight - lane.scrollTop - lane.clientHeight
  const scrollTo = (top: number) => { lane.scrollTop = top; lane.dispatchEvent(new Event('scroll')) }
  const mountedBound = async () => { await expect(Number(root.getAttribute('data-loaded'))).toBeGreaterThanOrEqual(2100); await expect(lane.querySelectorAll('[data-testid="transcript-turn"]').length).toBeLessThan(120) }
  const deliver = async (start: number) => {
    await waitFor(() => expect(root).toHaveAttribute('data-page', 'Loading'))
    canvas.getByRole('button', { name: 'Deliver older page' }).click()
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
  await waitFor(() => expect(root.getAttribute('data-jump-result'), 'History setup: checkpoint jump resolves Anchored').toBe('Anchored'), { timeout: 5000 })
  await waitFor(() => expect(row(1300)).not.toBeNull())
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  const anchor = row(1300)!
  const readingTop = row(1300)!.getBoundingClientRect().top - lane.getBoundingClientRect().top
  const heldReader = within(anchor).getByRole('button', { name: /Worked/ })
  heldReader.focus({ preventScroll: true })
  // Hold a real row pointer down through the asynchronous host append: focus alone
  // does not exercise the viewport's pointer-held pin.
  heldReader.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1, button: 0, buttons: 1, isPrimary: true }))
  try {
    canvas.getByRole('button', { name: 'Append turn' }).click()
    await waitFor(() => expect(root).toHaveAttribute('data-end', '2402'))
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
    await waitFor(() => expect(row(1300)!.getBoundingClientRect().top - lane.getBoundingClientRect().top - readingTop).toBe(0))
    await expect(bottomGap()).toBeGreaterThan(lane.clientHeight)
    await expect(document.activeElement).toBe(heldReader)
    await expect(canvas.getByRole('button', { name: /New messages/ })).toBeVisible()
  } finally {
    // Release off the disclosure: the hold is a reading gesture, not a press.
    // Releasing on the button would legitimately toggle it whenever its node
    // survived the append, leaving the later fold step in an arbitrary state.
    document.body.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerId: 1, button: 0, buttons: 0, isPrimary: true }))
  }
  await expect(within(row(1300)!).getByRole('button', { name: /Worked/ }), 'History tour: held reader did not press its disclosure').toHaveAttribute('aria-expanded', 'false')

  // Remeasure wrapping and theme without moving the current reading anchor.
  for (const name of ['Resize reading lane', 'Switch reading theme', 'Resize reading lane', 'Switch reading theme']) {
    const top = row(1300)!.getBoundingClientRect().top - lane.getBoundingClientRect().top
    await userEvent.click(canvas.getByRole('button', { name }))
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
    await waitFor(() => expect(row(1300)!.getBoundingClientRect().top - lane.getBoundingClientRect().top - top).toBe(0))
  }

  // A manually completed asynchronous prepend must move the reading anchor by 0px.
  await userEvent.click(canvas.getByRole('button', { name: 'Request older page' }))
  const focusedBeforePrepend = within(row(1300)!).getByRole('button', { name: /Worked/ })
  focusedBeforePrepend.focus({ preventScroll: true })
  await deliver(200)
  await waitFor(() => {
    const current = row(1300)
    expect(current !== null, 'History tour: prepended checkpoint remains render-ready').toBe(true)
    expect(current!.getBoundingClientRect().top - lane.getBoundingClientRect().top - readingTop, 'History tour: prepend anchor').toBe(0)
    expect(document.activeElement, 'History tour: prepend keeps focused element identity').toBe(focusedBeforePrepend)
  }, { timeout: 5000 })
  await expect(document.activeElement).toBe(focusedBeforePrepend)
  await mountedBound()

  // Expand and fold a measured mid-list work log, retaining its turn and reading position.
  const foldTop = row(1300)!.getBoundingClientRect().top - lane.getBoundingClientRect().top
  await waitFor(() => expect(getComputedStyle(within(row(1300)!).getByRole('button', { name: /Worked/ })).pointerEvents).not.toBe('none'))
  await userEvent.click(within(row(1300)!).getByRole('button', { name: /Worked/ }))
  await waitFor(() => expect(within(row(1300)!).getByRole('button', { name: /Worked/ })).toHaveAttribute('aria-expanded', 'true'))
  await waitFor(() => expect(row(1300)!.getBoundingClientRect().top - lane.getBoundingClientRect().top - foldTop).toBe(0))
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  const expandedHeight = row(1300)!.getBoundingClientRect().height
  await waitFor(() => expect(getComputedStyle(within(row(1300)!).getByRole('button', { name: /Worked/ })).pointerEvents).not.toBe('none'))
  await userEvent.click(within(row(1300)!).getByRole('button', { name: /Worked/ }))
  await waitFor(() => expect(within(row(1300)!).getByRole('button', { name: /Worked/ })).toHaveAttribute('aria-expanded', 'false'))
  await waitFor(() => expect(row(1300)!.getBoundingClientRect().height).toBeLessThan(expandedHeight))
  await waitFor(() => expect(row(1300)!.getBoundingClientRect().top - lane.getBoundingClientRect().top - foldTop).toBe(0))
  await mountedBound()

  // RAC row keyboard handling is exercised with real row focus, not a synthetic
  // scroller key default. Native scroller-focus keys use the design browser gate.
  // Start the keyboard slice from a fresh host navigation after the preceding
  // width/theme/disclosure changes; no key default or scroll amount is emulated.
  // Checkpoint 1302 has one answer paragraph and no code fence at either width.
  // RAC may correctly retain the same key when the current row is taller than a page.
  await userEvent.click(canvas.getByRole('button', { name: 'Jump to loaded checkpoint 1302' }))
  await waitFor(() => expect(row(1302) !== null, 'History keyboard setup: current short turn').toBe(true))
  await expect(row(1302)!.getBoundingClientRect().height, 'History keyboard setup: short row fits viewport').toBeLessThan(lane.clientHeight)
  await focusHistoryKeyboardRow(lane, 1302)
  await new Promise<void>(resolve => setTimeout(resolve, 200))
  const beforePageUp = lane.scrollTop
  await userEvent.keyboard('{PageUp}')
  await waitFor(() => expect(lane.scrollTop).toBeLessThan(beforePageUp))
  // Empty success retains the oldest ID and must not trigger a request loop.
  await focusHistoryKeyboardRow(lane)
  await userEvent.keyboard('{Home}')
  await waitFor(() => expect(root).toHaveAttribute('data-page', 'Loading'))
  const requestsBeforeEmpty = root.getAttribute('data-requests')
  canvas.getByRole('button', { name: 'Deliver empty page' }).click()
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))))
  await expect(root).toHaveAttribute('data-page', 'Ready')
  await expect(root).toHaveAttribute('data-start', '200')
  await expect(root).toHaveAttribute('data-requests', requestsBeforeEmpty!)
  await userEvent.click(canvas.getByRole('button', { name: 'Fail next page' }))
  // Leaving and re-entering the boundary is new reading intent, not an automatic
  // empty-page loop: it must re-arm fetching without the fixture request button.
  await leaveHistoryBoundary(lane)
  await userEvent.keyboard('{Home}')
  await waitFor(() => expect(root).toHaveAttribute('data-page', 'Loading'))
  await waitFor(() => expect(lane.scrollTop).toBe(0))
  await expect(canvas.getByRole('grid')).toHaveAttribute('aria-busy', 'true')
  await expect(Number(root.getAttribute('data-requests'))).toBe(Number(requestsBeforeEmpty) + 1)
  canvas.getByRole('button', { name: 'Deliver older page' }).click()
  await waitFor(() => expect(root).toHaveAttribute('data-page', 'Failed'))
  await expect(await canvas.findByTestId('history-error')).toHaveTextContent('Earlier messages could not be loaded')
  await expect(root).toHaveAttribute('data-start', '200')
  const failedRequests = Number(root.getAttribute('data-requests'))
  await focusHistoryKeyboardRow(lane)
  await userEvent.keyboard('{Home}')
  lane.dispatchEvent(new WheelEvent('wheel', { bubbles: true, deltaY: -120 }))
  lane.dispatchEvent(new Event('scroll'))
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  await expect(root).toHaveAttribute('data-page', 'Failed')
  await expect(Number(root.getAttribute('data-requests'))).toBe(failedRequests)
  const retryControl = canvas.getByTestId('history-retry')
  for (let index = 0; index < 60 && document.activeElement !== retryControl; index++) await userEvent.tab()
  await expect(retryControl).toHaveFocus()
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(root).toHaveAttribute('data-page', 'Loading'))
  await expect(Number(root.getAttribute('data-requests'))).toBe(failedRequests + 1)
  await deliver(100)
  await expect(canvas.queryByTestId('history-error')).toBeNull()

  // The host pages until the target is committed, then observes an Anchored jump.
  await userEvent.click(canvas.getByRole('button', { name: 'Jump to unloaded checkpoint 50' }))
  await deliver(0)
  await waitFor(() => expect(root).toHaveAttribute('data-jump-result', 'Anchored'))
  await waitFor(() => expect(row(50)).not.toBeNull())
  await waitFor(() => {
    const target = row(50)!.getBoundingClientRect(), viewport = lane.getBoundingClientRect()
    expect(target.top).toBeGreaterThanOrEqual(viewport.top)
    expect(target.top).toBeLessThan(viewport.bottom)
  })
  await expect(canvas.queryByTestId('history-start')).toBeNull()
  await focusHistoryKeyboardRow(lane, 50)
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
  await waitFor(() => expect(lane.scrollHeight - lane.scrollTop - lane.clientHeight).toBeLessThanOrEqual(1), { timeout: 5000 })
  for (const start of [200, 100, 0]) {
    if (start !== 200) {
      await waitFor(() => expect(root.getAttribute('data-page'), 'History detector: stale-jump').toBe('Ready'))
      canvas.getByRole('button', { name: 'Request older page' }).click()
    }
    // Host transitions must commit separately: a same-task Request+Deliver would
    // click a still-disabled delivery control and never adopt the next page.
    await waitFor(() => expect(root).toHaveAttribute('data-page', 'Loading'))
    await expect(canvas.getByRole('grid')).toHaveAttribute('aria-busy', 'true')
    canvas.getByRole('button', { name: 'Deliver older page' }).click()
    await waitFor(() => expect(root).toHaveAttribute('data-start', String(start)))
    await waitFor(() => expect(root.getAttribute('data-page'), 'History detector: stale-jump').toBe('Ready'))
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
    await waitFor(() => expect(lane.scrollHeight - lane.scrollTop - lane.clientHeight, 'History detector: stale-jump').toBeLessThanOrEqual(1), { timeout: 5000 })
  }
  await expect(lane.querySelector('[data-embrace-entry-id="history/50"]'), 'History detector: stale-jump').toBeNull()
}
export const NewerFollowDark1440: Story = { play: exerciseNewerFollow }
export const NewerFollowLight1440: Story = { args: { scheme: 'light' }, play: exerciseNewerFollow }
export const NewerFollowDark440: Story = { args: { narrow: true }, play: exerciseNewerFollow }
export const NewerFollowLight440: Story = { args: { scheme: 'light', narrow: true }, play: exerciseNewerFollow }

async function exerciseTouchRearm({ canvasElement }: { canvasElement: HTMLElement }) {
  const canvas = within(canvasElement)
  const root = canvasElement.querySelector('main')!
  const lane = await canvas.findByTestId('transcript-scroll')
  await waitFor(() => expect(lane.querySelector('[data-testid="transcript-turn"]')).not.toBeNull())
  const oldest = await settleInitialHistoryPage(canvasElement, lane)
  await focusHistoryKeyboardRow(lane)
  await userEvent.keyboard('{Home}')
  await waitFor(() => expect(root).toHaveAttribute('data-page', 'Loading'))
  await waitFor(() => expect(lane.scrollTop).toBe(0))
  await waitFor(() => expect(activeHistoryRow(lane, oldest) !== null, 'History setup: loaded head after RAC Home').toBe(true))
  await new Promise<void>(resolve => setTimeout(resolve, 200))
  await waitFor(() => expect(lane.scrollTop).toBe(0))
  const requests = Number(root.getAttribute('data-requests'))
  canvas.getByRole('button', { name: 'Deliver empty page' }).click()
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  await expect(root).toHaveAttribute('data-page', 'Ready')

  // Layout/measurement-driven scrolls inside the boundary are not fresh input.
  // Simulate that movement separately from the touch gesture that follows.
  lane.scrollTop = 32
  lane.dispatchEvent(new Event('scroll'))
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  lane.scrollTop = 16
  lane.dispatchEvent(new Event('scroll'))
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  await expect(root).toHaveAttribute('data-page', 'Ready')
  await expect(Number(root.getAttribute('data-requests'))).toBe(requests)
  await expect(lane.scrollTop).toBeLessThan(lane.clientHeight * 1.5)
  const touchGeometry = { scrollHeight: lane.scrollHeight, clientHeight: lane.clientHeight }

  // A downward-moving finger expresses older-history intent independently of
  // scroll movement. Keep browser pointer takeover before the real touchmove.
  lane.dispatchEvent(new TouchEvent('touchstart', { bubbles: true, touches: [new Touch({ identifier: 2, target: lane, clientY: 40 })] }))
  lane.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch', pointerId: 2, isPrimary: true, buttons: 1 }))
  // Browser takeover may cancel the pointer before touch scrolling begins.
  lane.dispatchEvent(new PointerEvent('pointercancel', { bubbles: true, pointerType: 'touch', pointerId: 2, isPrimary: true }))
  await expect(lane.scrollHeight).toBe(touchGeometry.scrollHeight)
  await expect(lane.clientHeight).toBe(touchGeometry.clientHeight)
  lane.dispatchEvent(new TouchEvent('touchmove', { bubbles: true, touches: [new Touch({ identifier: 2, target: lane, clientY: 80 })] }))
  try {
    await waitFor(() => expect(root).toHaveAttribute('data-page', 'Loading'))
    await expect(Number(root.getAttribute('data-requests'))).toBe(requests + 1)
    canvas.getByRole('button', { name: 'Deliver older page' }).click()
    await waitFor(() => expect(root).toHaveAttribute('data-start', String(Math.max(0, oldest - pageSize))))
  } finally {
    lane.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerType: 'touch', pointerId: 2, isPrimary: true, buttons: 0 }))
    lane.dispatchEvent(new Event('touchend', { bubbles: true }))
  }
}
export const TouchRearmDark1440: Story = { play: exerciseTouchRearm }
export const TouchRearmLight1440: Story = { args: { scheme: 'light' }, play: exerciseTouchRearm }
export const TouchRearmDark440: Story = { args: { narrow: true }, play: exerciseTouchRearm }
export const TouchRearmLight440: Story = { args: { scheme: 'light', narrow: true }, play: exerciseTouchRearm }

async function exerciseTouchAtZero({ canvasElement }: { canvasElement: HTMLElement }, detector: 'touch-latch' | 'touch-scroll-required' = 'touch-scroll-required') {
  const canvas = within(canvasElement)
  const root = canvasElement.querySelector('main')!
  const lane = await canvas.findByTestId('transcript-scroll')
  await waitFor(() => expect(lane.querySelector('[data-testid="transcript-turn"]')).not.toBeNull())
  const oldest = await settleInitialHistoryPage(canvasElement, lane)
  await focusHistoryKeyboardRow(lane)
  await userEvent.keyboard('{Home}')
  await waitFor(() => expect(root).toHaveAttribute('data-page', 'Loading'))
  await waitFor(() => expect(lane.scrollTop).toBe(0))
  await waitFor(() => expect(activeHistoryRow(lane, oldest) !== null, 'History setup: loaded head after RAC Home').toBe(true))
  await new Promise<void>(resolve => setTimeout(resolve, 200))
  await waitFor(() => expect(lane.scrollTop).toBe(0))
  canvas.getByRole('button', { name: 'Deliver empty page' }).click()
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  await expect(root).toHaveAttribute('data-page', 'Ready')
  await waitFor(() => expect(lane.scrollTop).toBe(0))
  const requests = Number(root.getAttribute('data-requests'))
  lane.dispatchEvent(new TouchEvent('touchstart', { bubbles: true, touches: [new Touch({ identifier: 3, target: lane, clientY: 100 })] }))
  try {
    // No scroll write/event: overscroll at zero cannot produce upward movement.
    lane.dispatchEvent(new TouchEvent('touchmove', { bubbles: true, touches: [new Touch({ identifier: 3, target: lane, clientY: 140 })] }))
    await waitFor(() => expect(root.getAttribute('data-page'), `History detector: ${detector}`).toBe('Loading'))
    await waitFor(() => expect(lane.scrollTop).toBe(0))
    await expect(Number(root.getAttribute('data-requests')), `History detector: ${detector}`).toBe(requests + 1)
    canvas.getByRole('button', { name: 'Deliver empty page' }).click()
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
    // The same touch sequence is consumed once even after another empty result.
    lane.dispatchEvent(new TouchEvent('touchmove', { bubbles: true, touches: [new Touch({ identifier: 3, target: lane, clientY: 180 })] }))
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
    await expect(root.getAttribute('data-page'), `History detector: ${detector}`).toBe('Ready')
    await expect(Number(root.getAttribute('data-requests')), `History detector: ${detector}`).toBe(requests + 1)
  } finally {
    lane.dispatchEvent(new TouchEvent('touchend', { bubbles: true, touches: [] }))
  }
}
export const TouchAtZeroDark1440: Story = { play: exerciseTouchAtZero }
export const TouchAtZeroLight1440: Story = { args: { scheme: 'light' }, play: exerciseTouchAtZero }
export const TouchAtZeroDark440: Story = { args: { narrow: true }, play: exerciseTouchAtZero }
export const TouchAtZeroLight440: Story = { args: { scheme: 'light', narrow: true }, play: exerciseTouchAtZero }
function HistoryStates({ scheme = 'dark', narrow = false }: { scheme?: Scheme; narrow?: boolean }) {
  const [requested, setRequested] = React.useState(false)
  const [retried, setRetried] = React.useState(false)
  const examples: readonly { readonly label: string; readonly history: TranscriptHistory }[] = [
    { label: 'Complete', history: { _tag: 'Complete' } },
    { label: 'More', history: { _tag: 'More', loadOlder: async () => { setRequested(true) } } },
    { label: 'Loading', history: { _tag: 'Loading' } },
    { label: 'Failed', history: { _tag: 'Failed', retry: async () => { setRetried(true) } } },
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
  const moreExample = canvas.getByRole('region', { name: 'More history example' })
  moreExample.scrollIntoView({ block: 'center' })
  const more = within(moreExample).getByTestId('transcript-scroll')
  await focusHistoryKeyboardRow(more)
  await userEvent.keyboard('{Home}')
  await waitFor(() => expect(canvas.getByLabelText('History actions')).toHaveTextContent('Older page requested.'))
} }
export const AllStatesLight: Story = { ...AllStates, args: { scheme: 'light' } }

const contractFailures = ['pending-default', 'pending-explicit', 'jump-anchored', 'jump-missing', 'jump-superseded'] as const
type ContractFailure = typeof contractFailures[number]

function ContractFixture({ target, failure = 'none', scheme = 'dark', narrow = false }: { target: ContractFailure; failure?: Failure; scheme?: Scheme; narrow?: boolean }) {
  const handle = React.useRef<TranscriptHandle>(null)
  const committed = React.useRef(0)
  const observedResult = React.useRef('Waiting')
  const [revision, setRevision] = React.useState(0)
  const [complete, setComplete] = React.useState(false)
  const [result, setResult] = React.useState('')
  const [resultCommit, setResultCommit] = React.useState(-1)
  const [synchronous, setSynchronous] = React.useState('')
  const pending = target === 'pending-default' || target === 'pending-explicit'
  React.useLayoutEffect(() => { committed.current = revision }, [revision])
  const run = () => {
    observedResult.current = 'Waiting'
    setResult('Waiting')
    const id = target === 'jump-missing' && failure !== target ? 'history/absent/answer' : 'history/1300/read'
    const promise = handle.current!.jumpTo(id)
    void promise.then(outcome => { observedResult.current = outcome._tag; setResult(outcome._tag); setResultCommit(committed.current) })
    setSynchronous(observedResult.current)
    if ((target === 'jump-superseded' && failure !== target) || (target === 'jump-anchored' && failure === target)) {
      void handle.current!.jumpTo('history/1302/answer')
    }
    setRevision(value => value + 1)
  }
  const history: TranscriptHistory | undefined = complete || (pending && failure === target)
    ? { _tag: 'Complete' }
    : target === 'pending-default' ? undefined : { _tag: 'Pending' }
  const loaded = React.useMemo(() => turns.slice(1200, 1400), [])
  const messages = React.useMemo(() => loaded.flatMap(turn => [turn.prompt!, ...turn.items]), [loaded])
  return <main data-result={result} data-result-commit={resultCommit} data-call-stack={synchronous} data-revision={revision} {...stylex.props(styles.root, narrow && styles.narrow, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <div {...stylex.props(styles.controls)}>
      <button type="button" onClick={() => setComplete(true)}>Observe complete history</button>
      <button type="button" onClick={run}>Run jump contract</button>
    </div>
    <section aria-label="Promise contract history" {...stylex.props(styles.frame)}>
      <EmbraceRuntimeProvider options={{ messages, onNew: async () => {} }}>
        <Transcript ref={handle} title="Promise contract review" turns={loaded} history={history} sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} onOpenTool={() => {}} />
      </EmbraceRuntimeProvider>
    </section>
  </main>
}

async function exerciseContract(canvasElement: HTMLElement, target: ContractFailure) {
  const canvas = within(canvasElement)
  const root = canvasElement.querySelector('main')!
  const lane = await canvas.findByTestId('transcript-scroll')
  const detector = `History detector: ${target}`
  await waitFor(() => expect(lane.querySelector('[data-testid="transcript-turn"]')).not.toBeNull())
  if (target === 'pending-default' || target === 'pending-explicit') {
    await focusHistoryKeyboardRow(lane)
    await userEvent.keyboard('{Home}')
    await waitFor(() => expect(lane.scrollTop).toBe(0))
    lane.dispatchEvent(new WheelEvent('wheel', { bubbles: true, deltaY: -120 }))
    lane.dispatchEvent(new Event('scroll'))
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
    await expect(canvas.queryByTestId('history-start') !== null, detector).toBe(false)
    await expect(canvas.queryByTestId('history-loading') !== null, detector).toBe(false)
    await expect(canvas.queryByTestId('history-error') !== null, detector).toBe(false)
    await expect(canvas.queryByTestId('history-retry') !== null, detector).toBe(false)
    await expect(canvas.getByRole('grid'), detector).toHaveAttribute('aria-busy', 'false')
    await userEvent.click(canvas.getByRole('button', { name: 'Observe complete history' }))
    await expect(await canvas.findByTestId('history-start'), detector).toHaveTextContent('Start of conversation')
    return
  }
  await userEvent.click(canvas.getByRole('button', { name: 'Run jump contract' }))
  const expected = target === 'jump-anchored' ? 'Anchored' : target === 'jump-missing' ? 'Missing' : 'Superseded'
  await waitFor(() => expect(root.getAttribute('data-result'), detector).toBe(expected))
  await expect(root, detector).toHaveAttribute('data-call-stack', 'Waiting')
  if (target === 'jump-missing') await expect(root, detector).toHaveAttribute('data-result-commit', '1')
  if (target === 'jump-anchored') {
    await waitFor(() => expect(activeHistoryRow(lane, 1300), detector).not.toBeNull())
    await expect(within(activeHistoryRow(lane, 1300)!).getByRole('button', { name: /Worked/ }), detector).toHaveAttribute('aria-expanded', 'false')
    const row = activeHistoryRow(lane, 1300)!.getBoundingClientRect()
    const viewport = lane.getBoundingClientRect()
    await expect(row.top, detector).toBeGreaterThanOrEqual(viewport.top)
    await expect(row.top, detector).toBeLessThan(viewport.bottom)
  }
}

export const PendingDefaultProbe: Story = { render: args => <ContractFixture {...args} target="pending-default" />, play: ({ canvasElement }) => exerciseContract(canvasElement, 'pending-default') }
export const PendingExplicitProbe: Story = { render: args => <ContractFixture {...args} target="pending-explicit" />, play: ({ canvasElement }) => exerciseContract(canvasElement, 'pending-explicit') }
export const JumpAnchoredProbe: Story = { render: args => <ContractFixture {...args} target="jump-anchored" />, play: ({ canvasElement }) => exerciseContract(canvasElement, 'jump-anchored') }
export const JumpMissingProbe: Story = { render: args => <ContractFixture {...args} target="jump-missing" />, play: ({ canvasElement }) => exerciseContract(canvasElement, 'jump-missing') }
export const JumpSupersededProbe: Story = { render: args => <ContractFixture {...args} target="jump-superseded" />, play: ({ canvasElement }) => exerciseContract(canvasElement, 'jump-superseded') }
