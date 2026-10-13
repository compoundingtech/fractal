import type * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, waitFor, within } from 'storybook/test'
import { Transcript, type TranscriptTurn } from './assistant-ui/composition/Transcript'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import type { ConversationItem, TextItem } from './assistant-ui/embrace-data/model'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { geometryVars as g, surfaceVars as surface, spaceVars as s, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const at = '2032-01-01T12:00:00Z'
const hostPrompt: TextItem & { role: 'user' } = { _tag: 'Text', id: 'render/host-prompt', role: 'user', text: 'A prompt the host presents itself.', streaming: false, attachments: [], at }
const hostAnswer: TextItem = { _tag: 'Text', id: 'render/host-answer', role: 'assistant', text: 'An answer the host presents itself.', streaming: false, attachments: [], at }
const kitPrompt: TextItem & { role: 'user' } = { ...hostPrompt, id: 'render/kit-prompt', text: 'A prompt the host leaves to the kit.' }
const kitAnswer: TextItem = { ...hostAnswer, id: 'render/kit-answer', text: 'An answer the host leaves to the kit.' }
const messages: readonly TextItem[] = [hostPrompt, hostAnswer, kitPrompt, kitAnswer]
const work = { calls: [], durationMs: undefined, running: false, failed: false, interrupted: false }
const hosted: Readonly<Record<string, true>> = { [hostPrompt.id]: true, [hostAnswer.id]: true }
const styles = stylex.create({
  root: { width: g.polishWide, maxWidth: '100%', height: g.transcriptFrameHeight, backgroundColor: surface.canvas, padding: s.md },
  narrow: { width: g.polishNarrow },
  hosted: { margin: 0, color: ink.fgSoft, fontSize: t.uiSize, lineHeight: t.uiLeading },
})
function renderHosted(item: ConversationItem): React.ReactNode | undefined {
  if (hosted[item.id] !== true || item._tag !== 'Text') return undefined
  return <p data-testid="host-rendered" {...stylex.props(styles.hosted)}>Host view: {item.text}</p>
}
const meta = {
  title: 'Fractal UI/Transcript/Host item rendering', component: Transcript,
  parameters: { layout: 'fullscreen' },
  args: { title: 'Example host item rendering', turns, sync: { _tag: 'Live', since: Date.parse(at) }, now: Date.parse(at), observedAt: Date.parse(at), history: { _tag: 'Complete' }, renderItem: renderHosted, scheme: 'dark' as Scheme, narrow: false },
  argTypes: { scheme: { control: 'radio', options: ['light', 'dark'] }, narrow: { control: 'boolean' } },
  render: ({ scheme, narrow, ...args }) => <main {...stylex.props(styles.root, narrow && styles.narrow, ...baselineTheme, scheme === 'light' && lightTheme)}><EmbraceRuntimeProvider options={{ messages, isRunning: false, onNew: async () => {} }}><Transcript {...args} /></EmbraceRuntimeProvider></main>,
} satisfies Meta<React.ComponentProps<typeof Transcript> & { scheme: Scheme; narrow: boolean }>
export default meta
type Story = StoryObj<typeof meta>
export const Dark1440: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const prompt = await waitFor(() => { const node = canvasElement.querySelector(`[data-item-id="${hostPrompt.id}"]`); if (!(node instanceof HTMLElement)) throw new Error('host prompt not rendered'); return node })
    await expect(prompt).toHaveAttribute('data-testid', 'user-message')
    await expect(within(prompt).getByTestId('host-rendered')).toHaveTextContent(`Host view: ${hostPrompt.text}`)
    const answer = canvasElement.querySelector(`[data-item-id="${hostAnswer.id}"]`) as HTMLElement
    await expect(answer).toHaveAttribute('data-testid', 'transcript-message')
    await expect(answer).toHaveAttribute('data-item-kind', 'Text')
    await expect(within(answer).getByTestId('host-rendered')).toHaveTextContent(`Host view: ${hostAnswer.text}`)
    await expect(canvas.getAllByTestId('host-rendered')).toHaveLength(2)
    // Negative controls: items the host returns undefined for keep the kit presentation, never the host view.
    const kitUser = canvasElement.querySelector(`[data-item-id="${kitPrompt.id}"]`) as HTMLElement
    await expect(kitUser).toHaveAttribute('data-send-state', 'sent')
    await expect(kitUser).toHaveTextContent(kitPrompt.text)
    await expect(within(kitUser).queryByTestId('host-rendered')).toBeNull()
    const kitSeat = canvasElement.querySelector(`[data-item-id="${kitAnswer.id}"]`) as HTMLElement
    await expect(kitSeat).toHaveTextContent(kitAnswer.text)
    await expect(kitSeat).not.toHaveAttribute('data-testid', 'transcript-message')
    await expect(within(kitSeat).queryByTestId('host-rendered')).toBeNull()
    await expect(canvas.queryByText(/unknown/i)).toBeNull()
  },
}
export const Light1440: Story = { ...Dark1440, args: { scheme: 'light' } }
export const Dark440: Story = { ...Dark1440, args: { narrow: true } }
export const Light440: Story = { ...Dark1440, args: { narrow: true, scheme: 'light' } }
