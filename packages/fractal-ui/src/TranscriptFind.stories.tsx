import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { EmbraceRuntimeProvider, Transcript, assistantDarkTheme, compositionLightTheme, liveAccentTheme, type TranscriptHistory, type TranscriptTurn } from './assistant-ui'
import { surfaceVars as surface } from './assistant-ui/composition-tokens.stylex'

const now = Date.parse('2026-11-07T12:00:00Z')
const at = new Date(now).toISOString()
const output = 'Scanning sidebar folders\nread 12 entries\nresolved 3 folders\nnothing to migrate\nno conflicts\nfolders fix applied to the sidebar tree'
const turnAt = (index: number): TranscriptTurn => ({
  id: `find-turn-${index}`,
  prompt: { _tag: 'Text', id: `find-prompt-${index}`, role: 'user', text: index % 4 === 0 ? `Check the sidebar folders again, pass ${index}.` : `Review step ${index}.`, attachments: [], streaming: false, at },
  items: [
    { _tag: 'ToolCall', id: `find-tool-${index}`, callId: `find-call-${index}`, name: 'read', input: { path: 'src/sidebar.ts' }, status: 'success', callSeen: true, at, result: { content: output, isError: false, at } },
    { _tag: 'Text', id: `find-answer-${index}`, role: 'assistant', text: `Step ${index} is done; the **sidebar** reads cleanly.`, attachments: [], streaming: false, at },
  ],
  work: { calls: [{ id: `find-tool-${index}`, kind: 'read', title: 'Read sidebar', status: 'success', startedAt: at, endedAt: at, detail: output }], durationMs: 2000, running: false, failed: false, interrupted: false },
})
const turns = Array.from({ length: 24 }, (_, index) => turnAt(index))
const loadOlder = fn()

const TranscriptFindStory = ({ scheme, history }: { readonly scheme: 'dark' | 'light'; readonly history: TranscriptHistory }) => <main {...stylex.props(scheme === 'light' ? compositionLightTheme : assistantDarkTheme, liveAccentTheme, styles.root)}>
  <EmbraceRuntimeProvider options={{ messages: turns.flatMap(turn => [turn.prompt!, ...turn.items]), isRunning: false, onNew: async () => {} }}>
    <Transcript turns={turns} history={history} title="Transcript search" sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} onOpenTool={() => {}} />
  </EmbraceRuntimeProvider>
</main>
const styles = stylex.create({ root: { height: '100vh', backgroundColor: surface.canvas } })
const meta = { excludeStories: ['scenarioExemptions'], title: 'Fractal UI/Transcript search', component: TranscriptFindStory, parameters: { scenario: { _tag: 'contract-fixture', reason: 'Explicit TranscriptTurn and history fixtures exercise the transcript search UI contract: shortcut and focus ownership, match navigation, loaded-history coverage and tool disclosure, in dark and light themes, independently of scenario wire projection.' } }, args: { scheme: 'dark', history: { _tag: 'Complete' } } } satisfies Meta<typeof TranscriptFindStory>
export default meta
type Story = StoryObj<typeof meta>

export const scenarioExemptions: readonly string[] = [
  'ExpandsMatchedTool:contract-fixture',
  'ManyMatches:contract-fixture',
  'ManyMatchesLight:contract-fixture',
  'NoMatch:contract-fixture',
  'NoMatchLight:contract-fixture',
  'Open:contract-fixture',
  'OpenLight:contract-fixture',
  'PartialCoverage:contract-fixture',
  'PartialCoverageLight:contract-fixture',
]

async function openSearch(canvasElement: HTMLElement, query?: string) {
  const canvas = within(canvasElement)
  const lane = await canvas.findByTestId('transcript-scroll')
  lane.focus()
  await userEvent.keyboard('{Control>}f{/Control}')
  const input = await canvas.findByRole('textbox', { name: 'Search transcript' })
  await expect(input).toHaveFocus()
  if (query !== undefined) await userEvent.keyboard(query)
  return { canvas, input, lane, box: canvas.getByTestId('transcript-find') }
}

export const Open: Story = { tags: ['critical'], play: async ({ canvasElement }) => {
  const { canvas, input, lane } = await openSearch(canvasElement)
  // Opening floats over the lane: nothing shifts, and there is no permanent history band.
  await expect(canvas.queryByTestId('transcript-history-summary')).toBeNull()
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(input).not.toBeInTheDocument())
  await expect(lane).toHaveFocus()
} }
export const OpenLight: Story = { ...Open, args: { scheme: 'light' } }

export const NoMatch: Story = { tags: ['critical'], play: async ({ canvasElement }) => {
  const { canvas } = await openSearch(canvasElement, 'zebra crossing')
  await waitFor(() => expect(canvas.getByTestId('transcript-find-count')).toHaveTextContent('No matches'))
  await expect(canvas.getByTestId('transcript-find-status')).toHaveTextContent('No matches')
  await expect(canvas.getByRole('button', { name: 'Next match' })).toBeDisabled()
} }
export const NoMatchLight: Story = { ...NoMatch, args: { scheme: 'light' } }

export const PartialCoverage: Story = { tags: ['critical'], args: { history: { _tag: 'HasOlder', onLoadEarlier: loadOlder } }, play: async ({ canvasElement }) => {
  loadOlder.mockClear()
  const { canvas, box } = await openSearch(canvasElement, 'folders')
  await waitFor(() => expect(box.dataset.matchCount).not.toBe('0'))
  await expect(box).toHaveAttribute('data-coverage', 'partial')
  await expect(canvas.getByTestId('transcript-find-coverage')).toHaveTextContent('Loaded entries only')
  await expect(canvas.getByTestId('transcript-find-status')).toHaveTextContent('Loaded entries only')
  await userEvent.click(canvas.getByRole('button', { name: 'Search full history' }))
  await waitFor(() => expect(loadOlder).toHaveBeenCalled())
} }
export const PartialCoverageLight: Story = { ...PartialCoverage, args: { ...PartialCoverage.args, scheme: 'light' } }

export const ManyMatches: Story = { tags: ['critical'], play: async ({ canvasElement }) => {
  const { canvas, box } = await openSearch(canvasElement, 'sidebar')
  await waitFor(() => expect(Number(box.dataset.matchCount)).toBeGreaterThan(50))
  const count = canvas.getByTestId('transcript-find-count')
  const start = count.textContent
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(count.textContent).not.toBe(start))
  await userEvent.keyboard('{Shift>}{Enter}{/Shift}')
  await waitFor(() => expect(count).toHaveTextContent(start!))
  // Highlights paint in place, on mounted rows only, without touching message DOM.
  await waitFor(() => expect(CSS.highlights.get('transcript-search-match')?.size ?? 0).toBeGreaterThan(0))
  await expect(CSS.highlights.get('transcript-search-current')?.size).toBe(1)
  // The rule ships in the global stylesheet, since the app's CSP refuses inline style elements.
  await expect(canvasElement.querySelector('style')).toBeNull()
  await expect(getComputedStyle(document.documentElement).getPropertyValue('--fractal-search-current')).not.toBe('')
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(CSS.highlights.get('transcript-search-match')?.size).toBe(0))
} }
export const ManyMatchesLight: Story = { ...ManyMatches, args: { scheme: 'light' } }

export const ExpandsMatchedTool: Story = { tags: ['critical'], play: async ({ canvasElement }) => {
  const { canvas } = await openSearch(canvasElement, 'folders fix applied')
  await waitFor(() => expect(canvasElement.querySelector('[data-testid="tool-detail-preview"]')).toHaveTextContent('folders fix applied to the sidebar tree'))
  await expect(canvas.getByTestId('transcript-find-count').textContent).toMatch(/^\d+\/24$/)
} }
