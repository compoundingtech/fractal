import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { WorkBlock } from './assistant-ui/taste/WorkBlock'
import { workLogTurnFromItems, type WorkAgent, type WorkCallFacts, type WorkKind } from './assistant-ui/taste/work-log'
import type { ToolCallItem } from './assistant-ui/embrace-data/model'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

/*
 * One settled run of 20 steps under four plan items. Step 4 fails (exit 2) and the seat retries it as step 5;
 * step 16 delegates to two agents. Durations add up to 1m 47s; the phases take 31s, 7s, 40s and 29s.
 */
const toolKinds: Readonly<Record<string, WorkKind>> = { read: 'read', bash: 'run', eval: 'eval', edit: 'edit', task: 'agent' }
const fetchToken = 'Fetch the review token', recordDecisions = 'Record review decisions', startWatcher = 'Start the comment watcher', briefAgents = 'Brief and notify agents'
const quick = 100
interface Step { readonly name: keyof typeof toolKinds; readonly phase: string; readonly input: Readonly<Record<string, unknown>>; readonly ms: number; readonly output?: string; readonly agents?: readonly WorkAgent[] }
const agentsDone: readonly WorkAgent[] = [
  { id: 'agent/profile', name: 'RenderProfile', task: 'Trace where a long transcript spends its render time', status: 'success', durationMs: 372000 },
  { id: 'agent/audit', name: 'ElementAudit', task: 'Classify every element still painted outside the kit', status: 'success', durationMs: 243000 },
]
const agentsLive: readonly WorkAgent[] = [
  { id: 'agent/profile', name: 'RenderProfile', task: 'Trace where a long transcript spends its render time', status: 'running', durationMs: 81000 },
  { id: 'agent/audit', name: 'ElementAudit', task: 'Classify every element still painted outside the kit', status: 'success', durationMs: 64000 },
]
const steps: readonly Step[] = [
  { name: 'read', phase: fetchToken, input: { i: 'token skill', path: 'skills/token-vault.md' }, ms: quick },
  { name: 'bash', phase: fetchToken, input: { i: 'find field', command: 'vault get svc-review-token --fields-only --reason "review token"' }, ms: 6000, output: 'username\ntoken' },
  { name: 'bash', phase: fetchToken, input: { i: 'find by title', command: 'vault get "Review" --kind login --fields-only --reason "review token"' }, ms: 8000, output: 'username\ntoken\nnotes' },
  { name: 'bash', phase: fetchToken, input: { i: 'list fields', command: 'vault get "Review" --kind login --fields-only --reason' }, ms: quick, output: 'usage: vault get <item> [--fields-only] [--reason TEXT]\nvault: --reason needs a value\nexit 2' },
  { name: 'bash', phase: fetchToken, input: { i: 'list fields json', command: 'vault get "Review" --kind login --fields-only --output json --reason "review token"' }, ms: quick, output: '["username","token","notes"]' },
  { name: 'bash', phase: fetchToken, input: { i: 'read token, test', command: 'umask 077; F=$RUNTIME_DIR/review.token; vault read "Review/token" > "$F"; curl -sf -H @"$F" https://api.example.test/ping' }, ms: 17000, output: 'pong' },
  { name: 'eval', phase: fetchToken, input: { code: "TOKEN_FILE = runtime_dir() / 'review.token'" }, ms: quick },
  { name: 'bash', phase: recordDecisions, input: { i: 'record q2', command: "cd review && cat >> decisions.md <<'EOF'\nq2: keep the summary row\nEOF" }, ms: 6000 },
  { name: 'bash', phase: recordDecisions, input: { i: 'record q3–q5', command: "cd review && cat >> decisions.md <<'EOF'\nq3–q5: phases on demand\nEOF" }, ms: 1000 },
  { name: 'edit', phase: startWatcher, input: { path: 'watch/comments.py' }, ms: quick },
  { name: 'bash', phase: startWatcher, input: { i: 'baseline', command: 'cd watch && python3 comments.py --baseline' }, ms: 40000, output: 'baseline: 214 comments' },
  { name: 'eval', phase: startWatcher, input: { code: "r = comments('threads?page=review-board'); print(r)" }, ms: quick, output: '{"threads": 12}' },
  { name: 'bash', phase: startWatcher, input: { i: 'comment watcher', command: 'cd watch && nohup python3 comments.py --interval 60 &' }, ms: quick },
  { name: 'bash', phase: briefAgents, input: { i: 'brief layout', command: 'notify send layout --file briefs/layout.md' }, ms: 2000, output: 'sent layout' },
  { name: 'eval', phase: briefAgents, input: { code: 'def prefix(thread, tag):\n    return f"{tag}: {thread}"' }, ms: 11000 },
  { name: 'task', phase: briefAgents, input: { i: 'profile + audit investigations' }, ms: quick, agents: agentsDone },
  { name: 'eval', phase: briefAgents, input: { code: 'await tool.write({"path": "agent://Pairing", "content": brief})' }, ms: 6000 },
  { name: 'bash', phase: briefAgents, input: { i: 'brief explores', command: 'notify send explores --file briefs/explores.md' }, ms: 7000, output: 'sent explores' },
  { name: 'bash', phase: briefAgents, input: { i: 'notify reviewers', command: 'notify send reviewers --file notify.md' }, ms: 2000, output: 'sent reviewers' },
  { name: 'bash', phase: briefAgents, input: { i: 'notify', command: 'for to in reviewer archivist; do notify send "$to" --file notify.md; done' }, ms: 1000, output: 'sent reviewer\nsent archivist' },
]
const endingOutput = 'resolving recipients\nqueued reviewer\nqueued archivist\nsent reviewer\nnotify: thread archivist: not found\nexit 1'
const started = Date.parse('2026-01-15T14:55:00Z')
const iso = (offset: number) => new Date(started + offset).toISOString()

type State = 'settled' | 'partial' | 'failed' | 'running'
/** The fixture runs through the production projection, so kinds, counts and durations come from `workLogTurnFromItems`. */
function fixture(state: State) {
  const shown = state === 'running' ? 18 : 20
  const items: ToolCallItem[] = []
  const facts: Record<string, WorkCallFacts> = {}
  let offset = 0, liveStart = 0
  steps.slice(0, shown).forEach((step, index) => {
    const id = `work/step-${index + 1}`
    const ends = state === 'failed' && index === 19
    const live = state === 'running' && index === 17
    const status = live ? 'running' : index === 3 || ends ? 'error' : 'success'
    if (live) liveStart = offset
    items.push({ _tag: 'ToolCall', id, callId: `call/${index + 1}`, name: step.name, input: step.input, status, callSeen: true, at: iso(offset), ...live ? {} : { result: { content: ends ? endingOutput : step.output ?? '', isError: status === 'error', at: iso(offset + step.ms) } } })
    facts[id] = { phase: step.phase, exitCode: index === 3 ? 2 : ends ? 1 : undefined, agents: step.agents === undefined ? undefined : state === 'running' ? agentsLive : step.agents }
    offset += step.ms
  })
  const turn = workLogTurnFromItems(items, {
    kindFor: name => toolKinds[name] ?? 'run', running: state === 'running', failed: state === 'failed', interrupted: false,
    durationMs: state === 'settled' || state === 'failed' ? offset : undefined, startedAt: iso(0),
    completeHistory: state !== 'partial', callFacts: call => facts[call.id],
  })
  return { turn, now: state === 'running' ? iso(liveStart + 4000) : undefined }
}

const styles = stylex.create({
  root: { width: g.polishWide, maxWidth: '100%', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  narrow: { width: g.polishNarrow },
  lane: { width: g.lane, maxWidth: '100%' },
})

function WorkBlockStory({ state = 'settled', scheme = 'dark', narrow = false }: { state?: State; scheme?: Scheme; narrow?: boolean }) {
  const { turn, now } = React.useMemo(() => fixture(state), [state])
  return <main {...stylex.props(styles.root, narrow && styles.narrow, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <div {...stylex.props(styles.lane)}><WorkBlock turn={turn} now={now} ariaLabel="Turn work" /></div>
  </main>
}

const meta = {
  title: 'Fractal UI/Work block', component: WorkBlockStory, parameters: { layout: 'fullscreen' },
  args: { state: 'settled', scheme: 'dark', narrow: false },
  argTypes: { state: { control: 'radio', options: ['settled', 'partial', 'failed', 'running'] }, scheme: { control: 'radio', options: ['dark', 'light'] }, narrow: { control: 'boolean' } },
} satisfies Meta<typeof WorkBlockStory>
export default meta
type Story = StoryObj<typeof meta>

const nextFrame = () => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
/**
 * Runs a passing check, applies one regression, requires the same check to reject it, restores, and re-asserts.
 * The regression is either a DOM change or a real press on the production component; `restore` undoes it the same way.
 */
async function withControl(check: () => Promise<void>, inject: () => Promise<void> | void, restore: () => Promise<void> | void) {
  await check()
  await inject()
  await nextFrame()
  let rejected = false
  try { await check() } catch { rejected = true }
  await restore()
  await nextFrame()
  await expect(rejected, 'the control regression must be rejected').toBe(true)
  await check()
}
const blockOf = (canvasElement: HTMLElement) => within(canvasElement).getByTestId('work-block')
const rowsOf = (block: HTMLElement, kind?: string) => Array.from(block.querySelectorAll<HTMLElement>(kind === undefined ? '[role="row"]:not([data-work-row="body"])' : `[role="row"][data-work-row="${kind}"]`))
const summaryButton = (block: HTMLElement) => rowsOf(block, 'summary')[0]!.querySelector<HTMLElement>('button')!
const phaseButton = (block: HTMLElement, index: number) => block.querySelector<HTMLElement>(`[data-work-row="phase"][data-phase-index="${index}"] button`)!
const stepRow = (block: HTMLElement, step: number) => block.querySelector<HTMLElement>(`[data-work-row="step"][data-call-id="work/step-${step}"]`)
const cell = (row: HTMLElement, name: string) => row.querySelector<HTMLElement>(`:scope > [data-cell="${name}"]`)!
const press = async (element: HTMLElement, expanded: 'true' | 'false') => { await userEvent.click(element); await waitFor(() => expect(element).toHaveAttribute('aria-expanded', expanded)) }
const box = (target: Element | Range) => target.getBoundingClientRect()
const same = async (values: readonly number[], what: string) => { for (const value of values) await expect(Math.abs(value - values[0]!), what).toBeLessThan(0.5) }
/** Plays leave no focus ring behind, so the settled frame shows the resting table. */
const restFocus = (canvasElement: HTMLElement) => { const active = canvasElement.ownerDocument.activeElement; if (active instanceof HTMLElement) active.blur() }

/** Collapsed default: a settled block is one summary row, and the failure it holds is a count in danger ink. */
async function assertCollapsed(block: HTMLElement) {
  await expect(rowsOf(block)).toHaveLength(1)
  await expect(summaryButton(block)).toHaveAttribute('aria-expanded', 'false')
  await expect(block.querySelector('[data-work-children]')).toBeNull()
}
/** Phase expansion: four plan phases with their kind counts, failures and times; every phase still shut. */
async function assertPhases(block: HTMLElement) {
  const phases = rowsOf(block, 'phase')
  await expect(phases.map(row => row.querySelector('[data-work-title]')!.textContent)).toEqual(['Fetch the review token', 'Record review decisions', 'Start the comment watcher', 'Brief and notify agents'])
  await expect(phases.map(row => row.querySelector('[data-work-counts]')!.textContent)).toEqual(['5 commands · 1 eval · 1 read', '2 commands', '2 commands · 1 eval · 1 edit', '4 commands · 2 evals · 2 agents'])
  await expect(phases.map(row => cell(row, 'time').textContent)).toEqual(['31s', '7s', '40s', '29s'])
  await expect(phases.map(row => cell(row, 'status').textContent)).toEqual(['1 failed', '', '', ''])
  await expect(rowsOf(block, 'step')).toHaveLength(0)
  for (const row of phases) await expect(row.querySelector('button')).toHaveAttribute('aria-expanded', 'false')
}
/** One shared grid: every row 24 tall, one edge per column at every depth, at least 8 between columns. */
async function assertColumns(block: HTMLElement, narrow: boolean) {
  const rows = rowsOf(block)
  const table = box(block.querySelector('[role="table"]')!)
  for (const row of rows) await expect(box(row).height, 'row height').toBe(24)
  await same(rows.map(row => box(cell(row, 'status')).right), 'status right edge')
  await same(rows.map(row => box(cell(row, 'disclosure')).left), 'disclosure left edge')
  await same([...rows.map(row => box(cell(row, 'time')).right), table.right], 'time right edge')
  for (const row of rows) {
    const time = cell(row, 'time')
    await expect(getComputedStyle(time).fontVariantNumeric).toBe('tabular-nums')
    await expect(getComputedStyle(time).textAlign).toBe('right')
    await expect(getComputedStyle(cell(row, 'status')).textAlign).toBe('right')
    if (time.textContent !== '') {
      const text = document.createRange()
      text.selectNodeContents(time)
      await expect(Math.abs(box(text).right - box(time).right)).toBeLessThan(1)
    }
  }
  const details = rows.flatMap(row => row.querySelector<HTMLElement>(':scope > [data-cell="detail"]') ?? [])
  await expect(details.length).toBeGreaterThan(0)
  const phaseCounts = rowsOf(block, 'phase').map(row => row.querySelector<HTMLElement>('[data-work-counts]')!)
  if (narrow) {
    // Below the lane minimum the detail folds into the opened body and phase titles keep the room their counts took.
    for (const detail of details) await expect(getComputedStyle(detail).display).toBe('none')
    for (const counts of phaseCounts) await expect(getComputedStyle(counts).display).toBe('none')
  } else {
    await same([...details.map(detail => box(detail).left), table.left + 256 + 8], 'detail left edge')
    for (const counts of phaseCounts) await expect(box(counts).width).toBeGreaterThan(0)
  }
  for (const row of rows) {
    const cells = Array.from(row.querySelectorAll<HTMLElement>(':scope > [data-cell]')).filter(element => getComputedStyle(element).display !== 'none')
    for (let index = 1; index < cells.length; index++) await expect(box(cells[index]!).left - box(cells[index - 1]!).right, `gap before ${cells[index]!.dataset['cell']}`).toBeGreaterThanOrEqual(7.5)
  }
}
/** The failure that ended the turn is open with its tail; the failure the seat moved past keeps its mark but stays shut. */
async function assertAutoOpen(block: HTMLElement, ending: number | undefined) {
  const earlier = stepRow(block, 4)!
  await expect(cell(earlier, 'status')).toHaveTextContent(/^Failed · exit 2$/)
  await expect(earlier.querySelector('button')).toHaveAttribute('aria-expanded', 'false')
  const bodies = Array.from(block.querySelectorAll<HTMLElement>('[data-work-row="body"]'))
  await expect(bodies.map(body => body.dataset['callId'])).toEqual(ending === undefined ? [] : [`work/step-${ending}`])
}

const play = (narrow: boolean): Story['play'] => async ({ canvasElement }) => {
  const block = blockOf(canvasElement)
  await expect(rowsOf(block, 'summary')[0]!.querySelector('[data-work-title]')).toHaveTextContent(/^Worked for 1m 47s$/)
  await expect(rowsOf(block, 'summary')[0]!.querySelector('[data-work-counts]')).toHaveTextContent(/^20 steps$/)
  await expect(cell(rowsOf(block, 'summary')[0]!, 'status')).toHaveTextContent(/^1 failed$/)
  await expect(getComputedStyle(cell(rowsOf(block, 'summary')[0]!, 'status')).color).not.toBe(getComputedStyle(cell(rowsOf(block, 'summary')[0]!, 'time')).color)
  // Control: opening the block must break the collapsed default; closing it restores it.
  await withControl(() => assertCollapsed(block), () => press(summaryButton(block), 'true'), () => press(summaryButton(block), 'false'))

  await press(summaryButton(block), 'true')
  await waitFor(() => expect(rowsOf(block, 'phase')).toHaveLength(4))
  // Control: opening a phase must break "phases only, every phase shut"; closing it restores it.
  await withControl(() => assertPhases(block), () => press(phaseButton(block, 0), 'true'), () => press(phaseButton(block, 0), 'false'))

  // Phase expansion: a phase opens into its own steps, one level in, and only those.
  await press(phaseButton(block, 0), 'true')
  await waitFor(() => expect(rowsOf(block, 'step')).toHaveLength(7))
  for (const row of rowsOf(block, 'step')) await expect(row.closest('[data-work-children]')).toHaveAttribute('data-work-children', 'phase:0')
  await expect(rowsOf(block, 'step').map(row => row.querySelector('[data-call-title]')!.textContent)).toEqual(['token skill', 'find field', 'find by title', 'list fields', 'list fields json', 'read token, test', 'Eval'])
  // Rows without an intent show the verb at the quieter ink; intents read at the subject ink.
  await expect(getComputedStyle(stepRow(block, 7)!.querySelector('[data-call-title]')!).color).not.toBe(getComputedStyle(stepRow(block, 6)!.querySelector('[data-call-title]')!).color)
  await expect(stepRow(block, 2)!.querySelector('[data-call-detail]')).toHaveTextContent(/^vault get svc-review-token/)
  await expect(cell(stepRow(block, 1)!, 'time')).toHaveTextContent(/^$/)
  await expect(cell(stepRow(block, 6)!, 'time')).toHaveTextContent(/^17s$/)

  // Deepest table: phases at depth 1, steps at depth 2, delegated agents at depth 3.
  await press(phaseButton(block, 3), 'true')
  await press(stepRow(block, 16)!.querySelector('button')!, 'true')
  await waitFor(() => expect(rowsOf(block, 'agent')).toHaveLength(2))
  // Control: one row laid out as its own flex line instead of the shared tracks must break the shared edges.
  const flexRow = stepRow(block, 3)!
  await withControl(() => assertColumns(block, narrow), () => { flexRow.style.display = 'flex' }, () => { flexRow.style.removeProperty('display') })

  // Control: opening the earlier failure must break "only the turn-ending failure opens"; closing it restores the rule.
  const earlier = stepRow(block, 4)!.querySelector<HTMLElement>('button')!
  await withControl(() => assertAutoOpen(block, undefined), () => press(earlier, 'true'), () => press(earlier, 'false'))
  restFocus(canvasElement)
}

export const SettledDark1440: Story = { play: play(false) }
export const SettledLight1440: Story = { args: { scheme: 'light' }, play: play(false) }
export const SettledDark440: Story = { args: { narrow: true }, play: play(true) }
export const SettledLight440: Story = { args: { narrow: true, scheme: 'light' }, play: play(true) }

/** Incomplete history keeps a working fold: the known counts plus a quiet tag, never a disabled header. */
const partialPlay: Story['play'] = async ({ canvasElement }) => {
  const block = blockOf(canvasElement)
  const summary = rowsOf(block, 'summary')[0]!
  await expect(summary.querySelector('[data-work-tag]')).toHaveTextContent(/^partial history$/)
  await expect(summary.querySelector('[data-work-title]')).toHaveTextContent(/^Worked$/)
  await expect(summary.querySelector('[data-work-counts]')).toHaveTextContent(/^20 steps$/)
  await expect(summaryButton(block)).toHaveAccessibleName('Worked 20 steps partial history 1 failed')
  await expect(block).not.toHaveTextContent(/unverified|Unknown/)
  await assertCollapsed(block)
  const foldWorks = async () => {
    const button = summaryButton(block)
    await userEvent.click(button)
    await waitFor(() => expect(button).toHaveAttribute('aria-expanded', 'true'), { timeout: 500 })
    await expect(rowsOf(block, 'phase')).toHaveLength(4)
    await userEvent.click(button)
    await waitFor(() => expect(button).toHaveAttribute('aria-expanded', 'false'), { timeout: 500 })
    await assertCollapsed(block)
  }
  // Control: a dead fold header (the old disabled fallback) must fail the round trip. Stopping every press
  // event at the button before React sees it emulates a header that ignores input.
  const dead = (event: Event) => { event.stopPropagation(); event.preventDefault() }
  const pressEvents = ['pointerdown', 'pointerup', 'mousedown', 'mouseup', 'click', 'keydown', 'keyup'] as const
  await withControl(foldWorks, () => { for (const type of pressEvents) summaryButton(block).addEventListener(type, dead, true) }, () => { for (const type of pressEvents) summaryButton(block).removeEventListener(type, dead, true) })
  restFocus(canvasElement)
}
export const PartialHistoryDark1440: Story = { args: { state: 'partial' }, play: partialPlay }
export const PartialHistoryLight1440: Story = { args: { state: 'partial', scheme: 'light' }, play: partialPlay }
export const PartialHistoryDark440: Story = { args: { state: 'partial', narrow: true }, play: partialPlay }
export const PartialHistoryLight440: Story = { args: { state: 'partial', narrow: true, scheme: 'light' }, play: partialPlay }

/** E32 as amended: the turn-ending failure opens its phase and its tail; the earlier failure stays shut. */
const failurePlay = (narrow: boolean): Story['play'] => async ({ canvasElement }) => {
  const block = blockOf(canvasElement)
  await expect(summaryButton(block)).toHaveAttribute('aria-expanded', 'true')
  await expect(cell(rowsOf(block, 'summary')[0]!, 'status')).toHaveTextContent(/^2 failed$/)
  const opened = async () => {
    await expect(rowsOf(block, 'phase').map(row => row.querySelector('button')!.getAttribute('aria-expanded'))).toEqual(['false', 'false', 'false', 'true'])
    await expect(cell(stepRow(block, 20)!, 'status')).toHaveTextContent(/^Failed · exit 1$/)
    await expect(stepRow(block, 20)!.querySelector('button')).toHaveAttribute('aria-expanded', 'true')
  }
  // Control: opening a settled phase must break "only the failing phase opens"; closing it restores it.
  await withControl(opened, () => press(phaseButton(block, 1), 'true'), () => press(phaseButton(block, 1), 'false'))
  const body = block.querySelector<HTMLElement>('[data-work-row="body"][data-call-id="work/step-20"]')!
  await expect(within(body).getByTestId('work-step-output')).toHaveTextContent(/^sent reviewer\s+notify: thread archivist: not found\s+exit 1$/)
  await expect(body.querySelector('[data-tail-hidden]')).toHaveTextContent(/^… 3 earlier lines$/)
  // The tail starts at the step's text column and runs to the table's right edge.
  const title = stepRow(block, 20)!.querySelector('[data-call-title]')!
  const frame = body.querySelector('[data-opened-body]')!
  await expect(Math.abs(box(frame).left - box(title).left)).toBeLessThan(0.5)
  await expect(Math.abs(box(frame).right - box(block.querySelector('[role="table"]')!).right)).toBeLessThan(0.5)
  await userEvent.click(within(body).getByRole('button', { name: 'Full output' }))
  await waitFor(() => expect(within(body).getByTestId('work-step-output')).toHaveTextContent(/^resolving recipients/))
  await expect(body.querySelector('[data-tail-hidden]')).toBeNull()
  await userEvent.click(within(body).getByRole('button', { name: 'Last lines' }))
  await waitFor(() => expect(within(body).getByTestId('work-step-output')).toHaveTextContent(/^sent reviewer/))
  await expect(body.querySelector('[data-tail-hidden]')).toHaveTextContent(/^… 3 earlier lines$/)

  await press(phaseButton(block, 0), 'true')
  const earlier = stepRow(block, 4)!.querySelector<HTMLElement>('button')!
  await withControl(() => assertAutoOpen(block, 20), () => press(earlier, 'true'), () => press(earlier, 'false'))
  const flexRow = stepRow(block, 20)!
  await withControl(() => assertColumns(block, narrow), () => { flexRow.style.display = 'flex' }, () => { flexRow.style.removeProperty('display') })
  // Leave the default frame: only the turn-ending failure's phase open.
  await press(phaseButton(block, 0), 'false')
  restFocus(canvasElement)
}
export const TurnEndingFailureDark1440: Story = { args: { state: 'failed' }, play: failurePlay(false) }
export const TurnEndingFailureLight1440: Story = { args: { state: 'failed', scheme: 'light' }, play: failurePlay(false) }
export const TurnEndingFailureDark440: Story = { args: { state: 'failed', narrow: true }, play: failurePlay(true) }
export const TurnEndingFailureLight440: Story = { args: { state: 'failed', narrow: true, scheme: 'light' }, play: failurePlay(true) }

/** Live work keeps itself visible: only the phase holding the live step opens, with the running agent listed. */
const runningPlay = (narrow: boolean): Story['play'] => async ({ canvasElement }) => {
  const block = blockOf(canvasElement)
  const summary = rowsOf(block, 'summary')[0]!
  await expect(summary.querySelector('[data-work-title]')).toHaveTextContent(/^Working for 1m 41s$/)
  await expect(summary.querySelector('[data-work-counts]')).toHaveTextContent(/^18 steps$/)
  await expect(cell(summary, 'status')).toHaveTextContent(/^Running$/)
  const live = async () => {
    await expect(rowsOf(block, 'phase').map(row => row.querySelector('button')!.getAttribute('aria-expanded'))).toEqual(['false', 'false', 'false', 'true'])
    await expect(rowsOf(block, 'step').map(row => row.dataset['callId'])).toEqual(['work/step-14', 'work/step-15', 'work/step-16', 'work/step-17', 'work/step-18'])
  }
  // Control: opening a settled phase must break "only the live phase opens"; closing it restores it.
  await withControl(live, () => press(phaseButton(block, 2), 'true'), () => press(phaseButton(block, 2), 'false'))
  await expect(cell(stepRow(block, 18)!, 'status')).toHaveTextContent(/^Running$/)
  await expect(cell(stepRow(block, 18)!, 'time')).toHaveTextContent(/^4s$/)
  await expect(rowsOf(block, 'agent').map(row => [row.querySelector('[data-call-title]')!.textContent, cell(row, 'status').textContent, cell(row, 'time').textContent])).toEqual([['RenderProfile', 'Running', '1m 21s'], ['ElementAudit', '', '1m 04s']])
  await expect(block.querySelector('[data-work-row="body"]')).toBeNull()
  const flexRow = stepRow(block, 18)!
  await withControl(() => assertColumns(block, narrow), () => { flexRow.style.display = 'flex' }, () => { flexRow.style.removeProperty('display') })
  restFocus(canvasElement)
}
export const RunningDark1440: Story = { args: { state: 'running' }, play: runningPlay(false) }
export const RunningLight1440: Story = { args: { state: 'running', scheme: 'light' }, play: runningPlay(false) }
export const RunningDark440: Story = { args: { state: 'running', narrow: true }, play: runningPlay(true) }
export const RunningLight440: Story = { args: { state: 'running', narrow: true, scheme: 'light' }, play: runningPlay(true) }
