import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { useArgs } from 'storybook/preview-api'
import { AgentMessage, OutgoingMessageRow } from './assistant-ui/AgentMessage'
import { EmbraceToolItem } from './assistant-ui/EmbraceToolCall'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import { Transcript } from './assistant-ui/composition/Transcript'
import { workLogTurnFromItems } from './assistant-ui/taste/work-log'
import type { TextItem } from './assistant-ui/embrace-data/model'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { darkTheme } from './assistant-ui/embrace-theme'
import { tokens } from './assistant-ui/embrace-tokens.stylex'
import { agentMessage, agentMessageVias, outgoingMessage, broadcastMessage, longAgentMessage, largeAgentMessage } from './agent-message-fixtures'

const meta = { title: 'Fractal UI/Agent Messages', component: AgentMessage, parameters: { layout: 'fullscreen' }, args: { ...agentMessage, onCopyId: fn(), onLoadFull: fn(), onOpenReply: fn() }, decorators: [(Story, context) => <main data-scheme={context.parameters.scheme ?? 'dark'} {...stylex.props(styles.root, ...baselineTheme, context.parameters.scheme === 'light' ? lightTheme : darkTheme, context.parameters.width === 1440 ? styles.wide : styles.narrow)}><Story /></main>] } satisfies Meta<typeof AgentMessage>
export default meta
type Story = StoryObj<typeof meta>
export const Short: Story = { play: async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByRole('article', { name: 'Message from Parent.Review' })).toHaveAttribute('data-harness-weight', 'card')
  await expect(canvas.getByText('Please review the patch')).toHaveProperty('tagName', 'STRONG')
  await expect(canvas.queryByRole('button', { name: 'Show full message' })).toBeNull()
  await expect(canvas.getByText('IRC')).toBeVisible()
  await userEvent.click(canvas.getByRole('button', { name: 'Copy id' }))
  await expect(args.onCopyId).toHaveBeenCalledWith('Review.Patch')
  await expect(args.onLoadFull).not.toHaveBeenCalled()
} }
export const Clipped: Story = { args: { body: { _tag: 'Preview', markdown: 'The release review covers keyboard focus, retained answers and…' } }, play: async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByText('Message preview')).toBeVisible()
  await expect(canvas.queryByRole('button', { name: 'Full message loaded' })).toBeNull()
  await userEvent.click(canvas.getByRole('button', { name: 'Show full message' }))
  await expect(args.onLoadFull).toHaveBeenCalledTimes(1)
  await expect(args.onCopyId).not.toHaveBeenCalled()
} }
export const Loading: Story = { args: { body: { _tag: 'Loading', markdown: 'The release review covers keyboard focus and…' } }, play: async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  const loading = canvas.getByRole('button', { name: 'Loading full message…' })
  await userEvent.click(loading)
  await expect(loading).toHaveFocus()
  await expect(args.onLoadFull).not.toHaveBeenCalled()
  await expect(canvas.getByText(/Fetching the full message/)).toBeVisible()
  await expect(canvas.queryByText('The full message is shown.')).toBeNull()
} }
export const LoadFailed: Story = { args: { body: { _tag: 'Failed', markdown: 'The release review covers keyboard focus and…', reason: 'The message service disconnected' } }, play: async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByRole('alert')).toHaveTextContent('The message service disconnected')
  await expect(canvas.getByText(/The release review covers/)).toBeVisible()
  await userEvent.click(canvas.getByRole('button', { name: 'Retry full message' }))
  await expect(args.onLoadFull).toHaveBeenCalledTimes(1)
  await expect(args.onCopyId).not.toHaveBeenCalled()
} }
export const Reply: Story = { args: { replyTo: 'Parent.Question', subject: 'Release review', arrivedWhileWorking: true }, play: async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByRole('heading', { name: 'Release review' })).toBeVisible()
  await expect(canvas.getByText('arrived while working')).toBeVisible()
  await userEvent.click(canvas.getByRole('button', { name: /Parent.*Question/ }))
  await expect(args.onOpenReply).toHaveBeenCalledWith('Parent.Question')
  await expect(args.onCopyId).not.toHaveBeenCalled()
} }
export const FromParent: Story = { args: { from: 'Parent', to: 'Parent.Child', via: { _tag: 'FromParent' } }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByText('from parent')).toBeVisible()
  await expect(canvasElement.querySelector('[data-agent-id="Parent.Child"]')).toHaveTextContent('Parent › Child')
  await expect(canvas.queryByText('teammate')).toBeNull()
} }
export const BroadcastCopy: Story = { args: { delivery: { _tag: 'BroadcastCopy', recipients: 3 } }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByText('broadcast · 3 seats')).toBeVisible()
  await expect(canvasElement.querySelectorAll('[data-agent-message-direction=\"incoming\"]')).toHaveLength(1)
  await expect(canvasElement.querySelector('[data-work-kind=\"messages-sent\"]')).toBeNull()
} }
export const Untitled: Story = { args: { subject: undefined, id: undefined, to: undefined, at: undefined }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByRole('article', { name: 'Message from Parent.Review' })).toBeVisible()
  await expect(canvas.queryByRole('heading')).toBeNull()
  await expect(canvas.queryByRole('button', { name: 'Copy id' })).toBeNull()
  await expect(canvasElement.querySelector('time')).toBeNull()
} }
export const AllStates: Story = { render: () => <>
  {agentMessageVias.map(via => <AgentMessage key={via._tag} {...agentMessage} via={via} onCopyId={fn()} />)}
  <AgentMessage {...agentMessage} body={{ _tag: 'Full', markdown: longAgentMessage }} />
  <AgentMessage {...agentMessage} body={{ _tag: 'Preview', markdown: 'The release report is ready; the retained preview ends here…' }} onLoadFull={fn()} />
  <AgentMessage {...agentMessage} body={{ _tag: 'Loading', markdown: 'Retained preview while loading…' }} onLoadFull={fn()} />
  <AgentMessage {...agentMessage} body={{ _tag: 'Failed', markdown: 'Retained preview after disconnect…', reason: 'The message service disconnected' }} onLoadFull={fn()} />
  <AgentMessage {...agentMessage} subject="Release review" replyTo="Parent.Question" arrivedWhileWorking onOpenReply={fn()} />
  <AgentMessage {...agentMessage} delivery={{ _tag: 'BroadcastCopy', recipients: 3 }} />
  <AgentMessage from="Review" via={{ _tag: 'Irc' }} body={{ _tag: 'Full', markdown: 'Ready for review.' }} />
  <OutgoingMessageRow {...outgoingMessage} />
  <OutgoingMessageRow {...broadcastMessage} />
  <OutgoingMessageRow {...broadcastMessage} recipients={[...broadcastMessage.recipients.slice(0, 2), { id: 'Docs', receipt: { _tag: 'Failed', reason: 'Recipient retired before delivery' } }]} />
</>, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvasElement.querySelectorAll('[data-harness-weight=\"card\"]')).toHaveLength(12)
  await expect(canvasElement.querySelectorAll('[data-harness-weight=\"row\"]')).toHaveLength(3)
  await expect(canvas.queryByText('End of the full release review.')).toBeNull()
  await expect(canvas.getByRole('alert')).toHaveTextContent('the preview is kept')
} }
export const OutgoingSingle: Story = { render: () => <OutgoingMessageRow {...outgoingMessage} />, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const trigger = canvas.getByRole('button', { name: /Review.*Please review the release report/ })
  await expect(trigger).toHaveAttribute('aria-expanded', 'false')
  await expect(canvas.queryByText('The patch keeps the existing protocol and updates the examples.')).toBeNull()
  await userEvent.click(trigger)
  await expect(trigger).toHaveAttribute('aria-expanded', 'true')
  await expect(canvas.getByText('The patch keeps the existing protocol and updates the examples.')).toBeVisible()
  await expect(trigger).toHaveFocus()
  await userEvent.keyboard('{Enter}')
  await expect(trigger).toHaveAttribute('aria-expanded', 'false')
  await expect(trigger).toHaveFocus()
  await expect(canvas.queryByText('The patch keeps the existing protocol and updates the examples.')).toBeNull()
} }
export const OutgoingBroadcast: Story = { render: () => <OutgoingMessageRow {...broadcastMessage} />, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByRole('button', { name: /3 seats/ })).toHaveAttribute('aria-expanded', 'false')
  await expect(canvas.getByText(/injected/)).toBeVisible()
  await expect(canvas.getByText(/queued/)).toBeVisible()
  await expect(canvas.getByText(/revived/)).toBeVisible()
  await expect(canvas.queryByText(/failed/)).toBeNull()
} }
export const OutgoingFailed: Story = { render: () => <OutgoingMessageRow {...outgoingMessage} recipients={[{ id: 'Review', receipt: { _tag: 'Failed', reason: 'Recipient retired before delivery' } }]} />, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const receipt = canvasElement.querySelector<HTMLElement>('[aria-label=\"Review · failed · Recipient retired before delivery\"]')!
  receipt.focus()
  await expect(receipt).toHaveFocus()
  await expect(receipt).toHaveAttribute('title', 'Recipient retired before delivery')
  await expect(canvas.queryByText('Review: Recipient retired before delivery')).toBeNull()
  await userEvent.click(canvas.getByRole('button', { name: /Please review the release report/ }))
  await expect(canvas.getByText('Review: Recipient retired before delivery')).toBeVisible()
  await expect(canvas.queryByText(/injected/)).toBeNull()
} }
const quotedPrompt: TextItem & { role: 'user' } = { _tag: 'Text', role: 'user', id: 'quoted-message', text: 'Explain this example: <smalltalk-message from="Review">Ready for review.</smalltalk-message>', attachments: [], streaming: false, at: '2026-10-11T15:40:00Z' }
const quotedWork = workLogTurnFromItems([quotedPrompt], { kindFor: () => 'read', running: false, failed: false, interrupted: false, completeHistory: true })
export const NegativeControls: Story = { render: () => <>
  <EmbraceRuntimeProvider options={{ messages: [quotedPrompt], isRunning: false, onNew: fn() }}><Transcript title="Quoted example" turns={[{ id: 'quote-turn', prompt: quotedPrompt, items: [], work: quotedWork }]} sync={{ _tag: 'Live', since: 0 }} now={0} /></EmbraceRuntimeProvider>
  <EmbraceToolItem item={{ _tag: 'ToolCall', id: 'file-write', callId: 'file-write', name: 'write', input: { path: 'notes/agent-plan.md', content: 'Keep the release plan beside the report.' }, status: 'success', callSeen: true, at: '2026-10-11T15:40:00Z' }} variant="cards" />
</>, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(await canvas.findByTestId('user-message')).toHaveTextContent('<smalltalk-message')
  await expect(canvas.queryByRole('article', { name: /Message from/ })).toBeNull()
  await expect(canvasElement.querySelector('[data-work-kind="messages-sent"]')).toBeNull()
  await expect(canvas.getByText(/notes\/agent-plan.md/)).toBeVisible()
} }
export const AllStatesLight440: Story = { ...AllStates, parameters: { scheme: 'light', width: 440 } }
export const AllStatesDark1440: Story = { ...AllStates, parameters: { scheme: 'dark', width: 1440 } }
export const AllStatesLight1440: Story = { ...AllStates, parameters: { scheme: 'light', width: 1440 } }
export const ShortLight440: Story = { ...Short, parameters: { scheme: 'light', width: 440 } }
export const ShortDark1440: Story = { ...Short, parameters: { scheme: 'dark', width: 1440 } }
export const ShortLight1440: Story = { ...Short, parameters: { scheme: 'light', width: 1440 } }
export const ClippedLight440: Story = { ...Clipped, parameters: { scheme: 'light', width: 440 } }
export const ClippedDark1440: Story = { ...Clipped, parameters: { scheme: 'dark', width: 1440 } }
export const ClippedLight1440: Story = { ...Clipped, parameters: { scheme: 'light', width: 1440 } }
export const NegativeControlsLight440: Story = { ...NegativeControls, parameters: { scheme: 'light', width: 440 } }
export const NegativeControlsDark1440: Story = { ...NegativeControls, parameters: { scheme: 'dark', width: 1440 } }
export const NegativeControlsLight1440: Story = { ...NegativeControls, parameters: { scheme: 'light', width: 1440 } }
export const Long: Story = { args: { body: { _tag: 'Full', markdown: longAgentMessage } }, play: async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  const trigger = canvas.getByRole('button', { name: 'Show full message' })
  await expect(trigger).toHaveAttribute('aria-expanded', 'false')
  await expect(canvas.queryByText('End of the full release review.')).toBeNull()
  await userEvent.click(trigger)
  await expect(canvas.getByText('End of the full release review.')).toBeVisible()
  await expect(trigger).toHaveFocus()
  await userEvent.keyboard('{Enter}')
  await expect(trigger).toHaveAttribute('aria-expanded', 'false')
  await expect(trigger).toHaveFocus()
  await expect(args.onLoadFull).not.toHaveBeenCalled()
} }
export const UnavailableFullMessage: Story = { args: { body: { _tag: 'Preview', markdown: 'The release report is ready…' }, onLoadFull: undefined, fullMessageUnavailableReason: 'Open the originating thread to read the full message.' }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const reason = canvas.getByText('Open the originating thread to read the full message.')
  await expect(canvas.getByRole('button', { name: 'Show full message' })).toBeDisabled()
  reason.focus()
  await expect(reason).toHaveFocus()
  await expect(canvas.getByRole('button', { name: 'Show full message' })).toHaveAccessibleDescription('Open the originating thread to read the full message.')
  await expect(canvas.queryByText('The full message is shown.')).toBeNull()
} }
export const FullMessageTransition: Story = {
  args: { body: { _tag: 'Preview', markdown: 'The release report is ready…' } },
  render: function FullMessageArgs(args) {
    const [, updateArgs] = useArgs()
    return <AgentMessage {...args} onLoadFull={() => { args.onLoadFull?.(); updateArgs({ body: { _tag: 'Full', markdown: 'The complete release report is now available.' } }) }} />
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const trigger = canvas.getByRole('button', { name: 'Show full message' })
    await userEvent.click(trigger)
    await waitFor(() => expect(canvas.getByText('The complete release report is now available.')).toBeVisible())
    await expect(trigger).toHaveTextContent('Full message loaded')
    await expect(trigger).toHaveFocus()
    await expect(trigger).toHaveAttribute('aria-disabled', 'true')
    await userEvent.click(trigger)
    await expect(args.onLoadFull).toHaveBeenCalledTimes(1)
    await expect(canvas.queryByText('Message preview')).toBeNull()
  },
}
export const FailedMessageTransition: Story = { ...FullMessageTransition, args: { body: { _tag: 'Failed', markdown: 'Retained release preview…', reason: 'The message service disconnected' } }, play: async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByRole('alert')).toHaveTextContent('The message service disconnected')
  const retry = canvas.getByRole('button', { name: 'Retry full message' })
  await userEvent.click(retry)
  await waitFor(() => expect(canvas.getByText('The complete release report is now available.')).toBeVisible())
  await expect(retry).toHaveFocus()
  await expect(canvas.queryByRole('alert')).toBeNull()
  await expect(args.onLoadFull).toHaveBeenCalledTimes(1)
} }
export const LongLight440: Story = { ...Long, parameters: { scheme: 'light', width: 440 } }
export const LongDark1440: Story = { ...Long, parameters: { scheme: 'dark', width: 1440 } }
export const LongLight1440: Story = { ...Long, parameters: { scheme: 'light', width: 1440 } }
export const FullMessageTransitionLight440: Story = { ...FullMessageTransition, parameters: { scheme: 'light', width: 440 } }
export const FullMessageTransitionDark1440: Story = { ...FullMessageTransition, parameters: { scheme: 'dark', width: 1440 } }
export const FullMessageTransitionLight1440: Story = { ...FullMessageTransition, parameters: { scheme: 'light', width: 1440 } }
export const SmallTalk: Story = { args: { via: { _tag: 'SmallTalk' } }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByText('Small Talk')).toBeVisible()
  await expect(canvas.queryByText('IRC')).toBeNull()
} }
export const Teammate: Story = { args: { via: { _tag: 'Teammate' } }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByText('teammate')).toBeVisible()
  await expect(canvas.queryByText('from parent')).toBeNull()
} }
export const InterAgent: Story = { args: { via: { _tag: 'InterAgent' } }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByText('inter-agent')).toBeVisible()
  await expect(canvas.queryByText('teammate')).toBeNull()
} }
export const OutgoingWithoutReceiptFacts: Story = { render: () => <OutgoingMessageRow {...outgoingMessage} recipients={[{ id: 'Review' }]} />, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByRole('button', { name: /Review.*Please review/ })).toHaveAttribute('aria-expanded', 'false')
  await expect(canvas.queryByText(/injected|queued|revived|failed/)).toBeNull()
} }
export const OutgoingSingleLight440: Story = { ...OutgoingSingle, parameters: { scheme: 'light', width: 440 } }
export const OutgoingSingleDark1440: Story = { ...OutgoingSingle, parameters: { scheme: 'dark', width: 1440 } }
export const OutgoingSingleLight1440: Story = { ...OutgoingSingle, parameters: { scheme: 'light', width: 1440 } }
export const OutgoingBroadcastLight440: Story = { ...OutgoingBroadcast, parameters: { scheme: 'light', width: 440 } }
export const OutgoingBroadcastDark1440: Story = { ...OutgoingBroadcast, parameters: { scheme: 'dark', width: 1440 } }
export const OutgoingBroadcastLight1440: Story = { ...OutgoingBroadcast, parameters: { scheme: 'light', width: 1440 } }
export const OutgoingFailedLight440: Story = { ...OutgoingFailed, parameters: { scheme: 'light', width: 440 } }
export const OutgoingFailedDark1440: Story = { ...OutgoingFailed, parameters: { scheme: 'dark', width: 1440 } }
export const OutgoingFailedLight1440: Story = { ...OutgoingFailed, parameters: { scheme: 'light', width: 1440 } }
export const BroadcastWithoutCount: Story = { render: () => <OutgoingMessageRow {...broadcastMessage} delivery={{ _tag: 'Broadcast' }} />, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByRole('button', { name: /Broadcast.*Please review/ })).toHaveAttribute('aria-expanded', 'false')
  await expect(canvas.queryByText('all seats')).toBeNull()
  await expect(canvas.queryByText('3 seats')).toBeNull()
} }
export const FetchedLongMessage: Story = {
  args: { body: { _tag: 'Preview', markdown: 'The release review is ready…' } },
  render: function FetchedLongArgs(args) {
    const [, updateArgs] = useArgs()
    return <AgentMessage {...args} onLoadFull={() => { args.onLoadFull?.(); updateArgs({ body: { _tag: 'Full', markdown: longAgentMessage } }) }} />
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const load = canvas.getByRole('button', { name: 'Show full message' })
    await expect(canvas.queryByText('End of the full release review.')).toBeNull()
    await userEvent.click(load)
    await waitFor(() => expect(canvas.getByText('End of the full release review.')).toBeVisible())
    await expect(canvas.getByRole('button', { name: 'Hide full message' })).toHaveAttribute('aria-expanded', 'true')
    await expect(load).toHaveTextContent('Full message loaded')
    await expect(load).toHaveFocus()
    await expect(canvas.queryByRole('button', { name: 'Show full message' })).toBeNull()
    await expect(args.onLoadFull).toHaveBeenCalledTimes(1)
  },
}
export const LargeBodies: Story = { render: () => <>
  <AgentMessage {...agentMessage} body={{ _tag: 'Full', markdown: largeAgentMessage }} />
  <OutgoingMessageRow {...outgoingMessage} message={largeAgentMessage} />
</>, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvasElement.querySelector('[data-message-renderer="plain"]')).toBeNull()
  await userEvent.click(canvas.getByRole('button', { name: 'Show full message' }))
  const full = canvas.getByRole('region', { name: 'Full message' })
  await expect(full.querySelector('[data-message-renderer="plain"] pre')?.textContent).toBe(largeAgentMessage)
  await expect(full.querySelector('[data-testid="markdown"]')).toBeNull()
  await expect(full.querySelector('[data-syntax-token]')).toBeNull()
  await userEvent.click(canvas.getByRole('button', { name: /Review.*Large report/ }))
  await expect(canvasElement.querySelectorAll('[data-message-renderer="plain"]')).toHaveLength(2)
  await expect(canvasElement.querySelectorAll('[data-message-renderer="plain"] pre')[1]?.textContent).toBe(largeAgentMessage)
  await expect(canvasElement.querySelector('[data-message-renderer="plain"] code')).toBeNull()
} }
export const BroadcastWithoutCountLight440: Story = { ...BroadcastWithoutCount, parameters: { scheme: 'light', width: 440 } }
export const BroadcastWithoutCountDark1440: Story = { ...BroadcastWithoutCount, parameters: { scheme: 'dark', width: 1440 } }
export const BroadcastWithoutCountLight1440: Story = { ...BroadcastWithoutCount, parameters: { scheme: 'light', width: 1440 } }
export const FetchedLongMessageLight440: Story = { ...FetchedLongMessage, parameters: { scheme: 'light', width: 440 } }
export const FetchedLongMessageDark1440: Story = { ...FetchedLongMessage, parameters: { scheme: 'dark', width: 1440 } }
export const FetchedLongMessageLight1440: Story = { ...FetchedLongMessage, parameters: { scheme: 'light', width: 1440 } }
export const LargeBodiesLight440: Story = { ...LargeBodies, parameters: { scheme: 'light', width: 440 } }
export const LargeBodiesDark1440: Story = { ...LargeBodies, parameters: { scheme: 'dark', width: 1440 } }
export const LargeBodiesLight1440: Story = { ...LargeBodies, parameters: { scheme: 'light', width: 1440 } }
const styles = stylex.create({ root: { minHeight: '100vh', boxSizing: 'border-box', padding: 16, display: 'flex', flexDirection: 'column', gap: 16, backgroundColor: tokens.canvas, color: tokens.ink, fontFamily: 'sans-serif' }, narrow: { width: 440, maxWidth: '100%' }, wide: { width: 1440, maxWidth: '100%' } })
