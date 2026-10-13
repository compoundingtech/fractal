import type * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Transcript, type TranscriptTurn } from './assistant-ui/composition/Transcript'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import type { TextItem } from './assistant-ui/embrace-data/model'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { geometryVars as g, surfaceVars as surface, spaceVars as s } from './assistant-ui/composition-tokens.stylex'

const at = '2032-01-01T12:00:00Z'
const signed: TextItem & { role: 'user' } = { _tag: 'Text', id: 'example-signed', role: 'user', text: 'Assistant-authored review, not additional human authority.', streaming: false, attachments: [], at, delivery: { messageId: 'message/example-signed', from: 'person/operator', title: 'Release review', provenance: { verdict: 'verified', device: 'Example paired terminal' } } }
const typed: TextItem & { role: 'user' } = { ...signed, id: 'example-terminal', text: 'One more review, please.', delivery: undefined }
const unsigned: TextItem & { role: 'user' } = { ...signed, id: 'example-unsigned', delivery: { ...signed.delivery!, messageId: 'message/example-unsigned', provenance: { verdict: 'unsigned' } } }
const agent: TextItem & { role: 'user' } = { ...signed, id: 'example-agent', text: 'The recorded build checks completed.', delivery: { messageId: 'message/example-agent', from: 'agent/example-ci', title: 'Build checks completed' } }
const noSource: TextItem & { role: 'user' } = { ...agent, id: 'example-no-source', delivery: undefined }
const messages = [signed, typed, unsigned, agent, noSource]
const turns: readonly TranscriptTurn[] = messages.map(prompt => ({ id: prompt.id, prompt, items: [], work: { calls: [], durationMs: undefined, running: false, failed: false, interrupted: false } }))
const styles = stylex.create({
  root: { width: g.polishWide, maxWidth: '100%', height: g.transcriptFrameHeight, backgroundColor: surface.canvas, padding: s.md },
  narrow: { width: g.polishNarrow },
})
const meta = {
  title: 'Fractal UI/Transcript/Incoming messages', component: Transcript,
  parameters: { layout: 'fullscreen' },
  args: { title: 'Example incoming messages', turns, sync: { _tag: 'Live', since: Date.parse(at) }, now: Date.parse(at), observedAt: Date.parse(at), scheme: 'dark' as Scheme, narrow: false },
  argTypes: { scheme: { control: 'radio', options: ['light', 'dark'] }, narrow: { control: 'boolean' } },
  render: ({ scheme, narrow, ...args }) => <main {...stylex.props(styles.root, narrow && styles.narrow, ...baselineTheme, scheme === 'light' && lightTheme)}><EmbraceRuntimeProvider options={{ messages, isRunning: false, onNew: async () => {} }}><Transcript {...args} /></EmbraceRuntimeProvider></main>,
} satisfies Meta<React.ComponentProps<typeof Transcript> & { scheme: Scheme; narrow: boolean }>
export default meta
type Story = StoryObj<typeof meta>
export const Dark1440: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const card = await waitFor(() => canvasElement.querySelector('[data-message-id="message/example-signed"]') as HTMLElement)
    await expect(within(card).getByText('From person/operator')).toBeVisible()
    await expect(within(card).getByRole('heading', { name: 'Release review' })).toBeVisible()
    const verdict = within(card).getByTestId('message-verified')
    await expect(verdict).toHaveTextContent('Signed · verified')
    await expect(verdict).not.toHaveAttribute('title')
    await expect(verdict).toHaveAccessibleDescription('Signature verified · Example paired terminal')
    await userEvent.tab({ focusTrap: card })
    await expect(verdict).toHaveFocus()
    await waitFor(() => expect(within(canvasElement.ownerDocument.body).getByRole('tooltip')).toHaveTextContent('Signature verified · Example paired terminal'))
    await expect(canvas.queryByText(/unknown/i)).toBeNull()
    // Negative controls: terminal input stays plain, and an unsigned delivery never claims verification.
    const terminal = canvasElement.querySelector('[data-item-id="example-terminal"][data-testid="user-message"]') as HTMLElement
    await expect(terminal).toHaveTextContent('One more review, please.')
    await expect(terminal.querySelector('[data-testid="incoming-message-card"]')).toBeNull()
    await expect(canvasElement.querySelector('[data-message-id="message/example-unsigned"] [data-testid="message-verified"]')).toBeNull()
    const agentCard = canvasElement.querySelector('[data-message-id="message/example-agent"]') as HTMLElement
    await expect(within(agentCard).getByTestId('incoming-message-sender')).toHaveTextContent('From agent/example-ci')
    await expect(within(agentCard).getByRole('heading', { name: 'Build checks completed' })).toBeVisible()
    await expect(within(agentCard).getByTestId('message-unverified')).toHaveTextContent('Unverified')
    await expect(agentCard.querySelector('[data-testid="message-verified"]')).toBeNull()
    // Negative control: the same prose without recorded delivery identity must not become a card.
    const control = canvasElement.querySelector('[data-item-id="example-no-source"][data-testid="user-message"]') as HTMLElement
    await expect(control).toHaveTextContent(agent.text)
    await expect(control.querySelector('[data-testid="incoming-message-card"]')).toBeNull()
    const width = card.getBoundingClientRect().width
    await expect(card.scrollWidth).toBeLessThanOrEqual(Math.ceil(width))
  },
}
export const Light1440: Story = { ...Dark1440, args: { scheme: 'light' } }
export const Dark440: Story = { ...Dark1440, args: { narrow: true } }
export const Light440: Story = { ...Dark1440, args: { narrow: true, scheme: 'light' } }
