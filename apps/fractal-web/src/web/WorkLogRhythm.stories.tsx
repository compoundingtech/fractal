import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { EmbraceRuntimeProvider, Transcript, assistantDarkTheme, compositionLightTheme, liveAccentTheme } from '@fractal/ui/assistant-ui'
import { surfaceVars as surface } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { ConversationItem, ToolCallItem } from '../conversation/model.ts'
import { mapConversationFeed } from './conversationTranscript.ts'
import { recordedToolDetail } from './semanticTool.tsx'

const at = (seconds: number) => `2026-10-08T12:00:${String(seconds).padStart(2, '0')}.000Z`
const tool = (id: string, seconds: number, name: string, input: Record<string, unknown>, content: string): ToolCallItem => ({ _tag: 'ToolCall', id: `rhythm/${id}`, callId: `call/${id}`, name, input, status: 'success', callSeen: true, at: at(seconds), result: { content, isError: false, at: at(seconds + 1) } })
const items: readonly ConversationItem[] = [
  { _tag: 'Text', id: 'rhythm/prompt', role: 'user', text: 'Ask the reviewer, wait for the job, then add the folders fix.', attachments: [], streaming: false, at: at(0) },
  tool('read', 1, 'read', { path: 'src/folders.ts' }, 'export const folders = []'),
  tool('list', 3, 'read', { path: 'src/sidebar.ts' }, 'export const sidebar = []'),
  tool('send', 5, 'bash', { command: 'st conversations send agent/example/Reviewer --from agent/example/Builder --idempotency-key rhythm-send-1 --subject "Recorded review" --body "Please inspect the **recorded review**." </dev/null 2>&1 | tail -1' }, 'Message sent.'),
  tool('wait', 7, 'wait', { jobs: ['bg_example'], timeout: 30 }, 'bg_example completed'),
  tool('fix', 9, 'bash', { command: 'printf "Add folders fix"' }, 'Add folders fix'),
  { _tag: 'Status', id: 'rhythm/status', status: 'completed', detail: 'Finished', at: at(11) },
]

/** Real app projection and semantic detail in the kit transcript, so the panel spacing is measured where people see it. */
const WorkLogRhythm = ({ scheme }: { readonly scheme: 'dark' | 'light' }) => {
  const state = mapConversationFeed({ _tag: 'Observed', freshness: 'live', value: { items, hasOlder: false, observation: { empty: false } } }, {})
  if (state._tag !== 'Observed') throw new Error('expected an observed transcript')
  return <main {...stylex.props(scheme === 'light' ? compositionLightTheme : assistantDarkTheme, liveAccentTheme, styles.root)}>
    <EmbraceRuntimeProvider options={{ messages: state.items, isRunning: false, onNew: async () => {} }}>
      <Transcript title="Work log rhythm" turns={state.turns} sync={{ _tag: 'Live', since: 1000 }} now={1000} observedAt={1000} onOpenTool={() => {}} renderToolDetail={(item, call) => recordedToolDetail(item, call, () => {})} />
    </EmbraceRuntimeProvider>
  </main>
}
const styles = stylex.create({ root: { height: '100vh', backgroundColor: surface.canvas } })
const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal App/Work log rhythm',
  component: WorkLogRhythm,
  args: { scheme: 'dark' },
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'A synthetic recorded turn, through the same feed projection and semantic tool detail the live pane uses, pins expanded work-log spacing.' } },
} satisfies Meta<typeof WorkLogRhythm>
export default meta
type Story = StoryObj<typeof meta>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Dark:contract-fixture',
  'Light:contract-fixture',
]

const play: Story['play'] = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await userEvent.click(await canvas.findByRole('button', { name: /^Worked/ }))
  const gaps = () => {
    const rows = [...canvasElement.querySelectorAll<HTMLElement>('[data-testid="work-log-children"] [data-tool-status]')]
    return rows.flatMap((row, index) => {
      const head = row.firstElementChild!.getBoundingClientRect(), panel = row.querySelector('[data-call-detail]')?.getBoundingClientRect(), next = rows[index + 1]?.firstElementChild?.getBoundingClientRect()
      return panel === undefined ? [] : [panel.top - head.bottom, ...(next === undefined ? [] : [next.top - panel.bottom])]
    })
  }
  // Every expanded preview opens; rows and panels keep one gap on both sides.
  await expect(gaps().length).toBeGreaterThan(3)
  await expect(new Set(gaps()).size).toBe(1)
  await expect(gaps()[0]).toBe(6)
  // The tool preview is the only card around a message: no second border inside it.
  const card = canvas.getByTestId('outgoing-message-card')
  await expect(getComputedStyle(card).borderTopWidth).toBe('0px')
  await expect(canvas.getByTestId('wait-row').querySelector('[aria-hidden="true"]')).toHaveTextContent('·')
}
export const Dark: Story = { args: { scheme: 'dark' }, play }
export const Light: Story = { args: { scheme: 'light' }, play }
