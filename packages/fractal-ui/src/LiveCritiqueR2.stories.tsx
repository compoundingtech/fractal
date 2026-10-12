import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { Button } from 'react-aria-components'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Transcript } from './assistant-ui/composition/Transcript'
import { ThinkingEntry } from './assistant-ui/composition/ThinkingEntry'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import { EmbraceComposer } from './assistant-ui/EmbraceComposer'
import { WorkLogV1 } from './assistant-ui/taste/WorkLogV1'
import type { WorkLogTurn } from './assistant-ui/taste/work-log'
import type { TextItem } from './assistant-ui/embrace-data/model'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const at = '2026-01-15T12:00:00Z'
const now = Date.parse(at) + 12000
const answer: TextItem & { role: 'assistant' } = { _tag: 'Text', id: 'critique/answer', role: 'assistant', attachments: [], at, streaming: false, text: Array.from({ length: 30 }, (_, index) => `Paragraph ${index + 1}: Keep the reading lane stable while reviewing the selected resource.`).join('\n\n') }
const next: TextItem & { role: 'assistant' } = { ...answer, id: 'critique/next', text: 'The selection and keyboard focus are preserved.' }
const work: WorkLogTurn = { running: true, failed: false, interrupted: false, durationMs: undefined, calls: [
  { id: 'critique/read', kind: 'read', title: 'Read selected resource', status: 'success', startedAt: at, endedAt: '2026-01-15T12:00:02Z', detail: 'The selected resource has a stable identity.' },
  { id: 'critique/run', kind: 'run', title: 'Check a longer command label with complete arguments', argsSummary: 'printf ready', status: 'success', startedAt: at, endedAt: '2026-01-15T12:00:12Z', detail: 'ready' },
  { id: 'critique/wait', kind: 'read', title: 'Await the next observation', status: 'running', startedAt: at },
] }
const hint = 'Messages can be sent once this conversation loads.'
const failures = ['none', 'answer-meta', 'cold-composer', 'tool-rows', 'thinking', 'row-fade'] as const
type Failure = typeof failures[number]
const styles = stylex.create({
  root: { width: g.polishWide, maxWidth: '100%', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  narrow: { width: g.polishNarrow },
  transcript: { height: g.transcriptFrameHeight },
  controls: { display: 'flex', gap: s.md, paddingBlock: s.lg },
})

function Critique({ scheme = 'dark', narrow = false, failure = 'none' }: { scheme?: Scheme; narrow?: boolean; failure?: Failure }) {
  const root = React.useRef<HTMLElement>(null)
  const [revision, rerender] = React.useState(0)
  // Deliberately restore one old behavior after the production tree re-renders.
  // Each selectable negative control must break its corresponding measured assertion.
  React.useLayoutEffect(() => {
    if (revision === 0 || root.current === null) return
    const element = <T extends HTMLElement>(selector: string) => root.current!.querySelector<T>(selector)!
    if (failure === 'answer-meta') { element('[data-testid="answer-meta"]').style.height = 'auto'; element('[data-testid="answer-meta"]').style.opacity = '1' }
    if (failure === 'cold-composer') element('[data-testid="composer-placeholder"]').textContent = 'Message'
    if (failure === 'tool-rows') { element('[data-row-disclosure]').parentElement!.style.display = 'flex'; element('[data-row-disclosure]').parentElement!.lastElementChild!.textContent = 'OK · 2s' }
    if (failure === 'thinking') element('[data-testid="thinking-body"]').style.borderLeftWidth = '0'
    if (failure === 'row-fade') element('[data-tool-status="success"] > button').style.transitionDuration = '0s'
  }, [failure, revision])
  return <main ref={root} data-revision={revision} {...stylex.props(styles.root, narrow && styles.narrow, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <div {...stylex.props(styles.controls)}><Button onPress={() => rerender(value => value + 1)}>Apply selected regression</Button><span>{failure}</span></div>
    <section aria-label="Answer example" {...stylex.props(styles.transcript)}><EmbraceRuntimeProvider options={{ messages: [answer, next], onNew: async () => {} }}><Transcript title="Review answers" now={now} observedAt={now} sync={{ _tag: 'Live', since: now }} turns={[{ id: 'critique', items: [answer, next], work: { ...work, calls: [], running: false } }]} /></EmbraceRuntimeProvider></section>
    <section aria-label="Tool row example"><WorkLogV1 turn={work} hideLiveRow /></section>
    <section aria-label="Thinking example"><ThinkingEntry text="Compare **reported rows** with the input, then review the retained selection." /></section>
    <section aria-label="Cold composer example"><EmbraceRuntimeProvider options={{ messages: [], isDisabled: true, onNew: async () => {} }}><EmbraceComposer variant="C1" disabledReason={hint} /></EmbraceRuntimeProvider></section>
  </main>
}
const meta = {
  title: 'Fractal UI/Live critique r2', component: Critique, parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark', narrow: false, failure: 'none' },
  argTypes: { scheme: { control: 'radio', options: ['light', 'dark'] }, narrow: { control: 'boolean' }, failure: { control: 'select', options: failures, description: 'Select one old behavior. Its production-component play must fail after Apply selected regression re-renders the tree.' } },
} satisfies Meta<typeof Critique>
export default meta
type Story = StoryObj<typeof meta>

export const Dark1440: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await canvas.findAllByTestId('agent-message')
  const thinking = await canvas.findByRole('button', { name: 'Thinking' })
  await userEvent.click(thinking)
  await expect(thinking).toHaveAttribute('aria-expanded', 'true')
  await userEvent.click(canvas.getByRole('button', { name: 'Apply selected regression' }))
  await waitFor(() => expect(canvasElement.querySelector('main')).toHaveAttribute('data-revision', '1'))
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches

  // 4: clipping-aware hidden geometry, then pointer and keyboard reveal.
  const answers = await canvas.findAllByTestId('agent-message')
  const metadata = within(answers[0]!).getByTestId('answer-meta')
  await expect(metadata.getBoundingClientRect().height).toBe(0)
  await expect(getComputedStyle(metadata).opacity).toBe('0')
  await expect(getComputedStyle(metadata).transitionDuration).toBe(reduced ? '0s' : '0.2s')
  await userEvent.hover(answers[0]!)
  await waitFor(() => expect(metadata.getBoundingClientRect().height).toBe(24))
  await waitFor(() => expect(getComputedStyle(metadata).opacity).toBe('1'))
  await userEvent.unhover(answers[0]!)
  await waitFor(() => expect(metadata.getBoundingClientRect().height).toBe(0))
  answers[0]!.focus()
  await waitFor(() => expect(metadata.getBoundingClientRect().height).toBe(24))
  await userEvent.tab()
  await expect(within(answers[0]!).getByRole('button', { name: 'Copy answer' })).toHaveFocus()
  canvas.getByRole('button', { name: 'Apply selected regression' }).focus()
  await waitFor(() => expect(metadata.getBoundingClientRect().height).toBe(0))

  // 9: a single compact shape; the cold reason belongs to the empty field.
  const cold = canvas.getByRole('region', { name: 'Cold composer example' })
  const composer = within(cold).getByTestId('kit-composer')
  await expect(within(cold).getByTestId('composer-placeholder')).toHaveTextContent(hint)
  await expect(composer.getBoundingClientRect().height).toBeLessThanOrEqual(50)
  await expect(within(cold).getByRole('textbox')).toBeDisabled()
  await expect(within(cold).getByRole('button', { name: 'Send' })).toBeDisabled()

  // 10, 12: fixed disclosure column independent of label and result width.
  const rows = canvasElement.querySelectorAll<HTMLElement>('[data-tool-status="success"] > button')
  await expect(rows).toHaveLength(2)
  await expect(rows[0]!).not.toHaveTextContent('OK')
  await expect(rows[0]!.lastElementChild).toHaveTextContent('2s')
  await expect(rows[1]!.lastElementChild).toHaveTextContent('12s')
  const columns = Array.from(rows, row => row.querySelector('[data-row-disclosure]')!.getBoundingClientRect().left)
  await expect(Math.abs(columns[0]! - columns[1]!)).toBeLessThan(1)
  await expect(canvasElement.querySelector('[data-tool-status="running"]')).toHaveTextContent('Running')
  await expect(getComputedStyle(rows[0]!).transitionDuration).toBe(reduced ? '0s' : '0.15s')
  await userEvent.click(rows[0]!)
  await expect(rows[0]!).toHaveAttribute('aria-expanded', 'true')
  await expect(canvas.getByTestId('work-call-output')).toHaveTextContent('stable identity')

  // 11: inspect actual rendered Markdown, not just its parent.
  await expect(parseFloat(getComputedStyle(thinking).fontSize)).toBe(14)
  const body = canvas.getByTestId('thinking-body')
  await expect(parseFloat(getComputedStyle(body).borderLeftWidth)).toBe(1)
  const paragraph = body.querySelector('p')!
  await expect(parseFloat(getComputedStyle(paragraph).fontSize)).toBe(13)
  await expect(getComputedStyle(paragraph).color).toBe(getComputedStyle(thinking).color)
  await expect(getComputedStyle(thinking).transitionDuration).toBe(reduced ? '0s' : '0.15s')
  await userEvent.click(thinking)
  await expect(canvas.queryByTestId('thinking-body')).toBeNull()
} }
export const Light1440: Story = { ...Dark1440, args: { scheme: 'light' } }
export const Dark440: Story = { ...Dark1440, args: { narrow: true } }
export const Light440: Story = { ...Dark1440, args: { narrow: true, scheme: 'light' } }
export const AllStates: Story = { ...Dark1440 }
