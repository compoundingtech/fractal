import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import axe from 'axe-core'
import { AgentMessage, OutgoingMessageRow } from './assistant-ui/AgentMessage'
import { EmbraceToolItem } from './assistant-ui/EmbraceToolCall'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import { Transcript } from './assistant-ui/composition/Transcript'
import { workLogTurnFromItems } from './assistant-ui/taste/work-log'
import type { TextItem, ToolCallItem } from './assistant-ui/embrace-data/model'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { darkTheme as legacyToolDarkTheme } from './assistant-ui/embrace-theme'
import { surfaceVars as surface, textVars as ink, typeVars as t, spaceVars as s, geometryVars as g } from './assistant-ui/composition-tokens.stylex'
import { agentMessage, agentMessageVias, outgoingMessage, broadcastMessage, longAgentMessage, largeAgentMessage } from './agent-message-fixtures'

type ContrastColor = readonly [number, number, number, number]
const parseContrastColor = (css: string): ContrastColor => {
  const values = css.match(/[0-9.]+/g)?.map(Number)
  if (values === undefined || values.length < 3) throw new Error(`Expected a computed RGB color, received ${css}`)
  return [values[0]!, values[1]!, values[2]!, values[3] ?? 1]
}
const blendContrastColor = (front: ContrastColor, back: ContrastColor): ContrastColor => [front[0] * front[3] + back[0] * (1 - front[3]), front[1] * front[3] + back[1] * (1 - front[3]), front[2] * front[3] + back[2] * (1 - front[3]), 1]
const contrastLuminance = (color: ContrastColor) => {
  const linear = [color[0], color[1], color[2]].map(value => { const channel = value / 255; return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4 })
  return linear[0]! * 0.2126 + linear[1]! * 0.7152 + linear[2]! * 0.0722
}
const contrastRatio = (front: ContrastColor, back: ContrastColor) => {
  const foreground = contrastLuminance(blendContrastColor(front, back)), background = contrastLuminance(back)
  return (Math.max(foreground, background) + 0.05) / (Math.min(foreground, background) + 0.05)
}
const computedTextContrast = (element: HTMLElement) => {
  const layers: ContrastColor[] = []
  for (let ancestor: HTMLElement | null = element; ancestor !== null; ancestor = ancestor.parentElement) {
    const color = parseContrastColor(getComputedStyle(ancestor).backgroundColor)
    layers.push(color)
    if (color[3] === 1) break
  }
  let background = layers.pop()
  if (background === undefined || background[3] !== 1) throw new Error('The specimen must have an opaque theme background')
  for (const layer of layers.reverse()) background = blendContrastColor(layer, background)
  return contrastRatio(parseContrastColor(getComputedStyle(element).color), background)
}

const meta = { title: 'Fractal UI/Agent Messages', component: AgentMessage, parameters: { layout: 'fullscreen' }, args: { ...agentMessage, onCopyId: fn(), onLoadFull: fn(), onOpenReply: fn() }, decorators: [(Story, context) => <main data-scheme={context.parameters.scheme ?? 'dark'} {...stylex.props(styles.root, ...baselineTheme, context.parameters.scheme === 'light' && lightTheme, context.parameters.width === 1440 ? styles.wide : styles.narrow)}><Story /></main>] } satisfies Meta<typeof AgentMessage>
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
  await expect(canvasElement.querySelectorAll('[data-harness-weight="card"]')).toHaveLength(15)
  await expect(canvasElement.querySelectorAll('[data-harness-weight="row"]')).toHaveLength(0)
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
export const NegativeControls: Story = { render: (_args, context) => <>
  <EmbraceRuntimeProvider options={{ messages: [quotedPrompt], isRunning: false, onNew: fn() }}><Transcript title="Quoted example" turns={[{ id: 'quote-turn', prompt: quotedPrompt, items: [], work: quotedWork }]} sync={{ _tag: 'Live', since: 0 }} now={0} observedAt={0} /></EmbraceRuntimeProvider>
  <div data-testid="legacy-file-control" {...stylex.props(context.parameters.scheme !== 'light' && legacyToolDarkTheme)}><EmbraceToolItem item={{ _tag: 'ToolCall', id: 'file-write', callId: 'file-write', name: 'write', input: { path: 'notes/agent-plan.md', content: 'Keep the release plan beside the report.' }, status: 'success', callSeen: true, at: '2026-10-11T15:40:00Z' }} variant="cards" /></div>
</>, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(await canvas.findByTestId('user-message')).toHaveTextContent('<smalltalk-message')
  await expect(canvas.queryByRole('article', { name: /Message from/ })).toBeNull()
  await expect(canvasElement.querySelector('[data-work-kind="messages-sent"]')).toBeNull()
  await expect(canvas.getAllByText(/notes\/agent-plan.md/).find(element => element.getClientRects().length > 0)).toBeVisible()
  const legacy = canvas.getByTestId('legacy-file-control')
  const markdown = within(legacy).getByText('Keep the release plan beside the report.', { selector: 'p' })
  await expect(computedTextContrast(markdown)).toBeGreaterThanOrEqual(4.5)
  // This is the prior white-on-white failure: the same checker must reject it.
  await expect(contrastRatio([245, 245, 245, 0.8], [255, 255, 255, 1])).toBeLessThan(4.5)
  const contrast = await axe.run(legacy, { runOnly: { type: 'rule', values: ['color-contrast'] } })
  await expect(contrast.violations).toEqual([])
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
    const [body, setBody] = React.useState(args.body)
    return <AgentMessage {...args} body={body} onLoadFull={() => { args.onLoadFull?.(); setBody({ _tag: 'Full', markdown: 'The complete release report is now available.' }) }} />
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
  await expect(canvas.getByRole('button', { name: /3 seats.*Please review/ })).toHaveAttribute('aria-expanded', 'false')
  await expect(canvas.queryByText('all seats')).toBeNull()
  await expect(canvas.queryByText('Broadcast')).toBeNull()
} }
export const FetchedLongMessage: Story = {
  args: { body: { _tag: 'Preview', markdown: 'The release review is ready…' } },
  render: function FetchedLongArgs(args) {
    const [body, setBody] = React.useState(args.body)
    return <AgentMessage {...args} body={body} onLoadFull={() => { args.onLoadFull?.(); setBody({ _tag: 'Full', markdown: longAgentMessage }) }} />
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
  await expect(full.querySelector('[data-message-window]')?.textContent).toBe(largeAgentMessage.slice(0, 16000))
  await expect(full.querySelector('[data-testid="markdown"]')).toBeNull()
  await expect(full.querySelector('[data-syntax-token]')).toBeNull()
  await userEvent.click(canvas.getByRole('button', { name: /Review.*Large report/ }))
  await expect(canvasElement.querySelectorAll('[data-message-renderer="plain"]')).toHaveLength(2)
  await expect(canvasElement.querySelectorAll('[data-message-renderer="plain"] pre')[1]?.textContent).toBe(largeAgentMessage.slice(0, 16000))
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
const multiMegabyteMessage = largeAgentMessage.repeat(20)
export const PagedMultiMegabyteMessage: Story = {
  render: () => <AgentMessage {...agentMessage} body={{ _tag: 'Full', markdown: multiMegabyteMessage }} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: 'Show full message' }))
    const next = canvas.getByRole('button', { name: 'Next message page' })
    const previous = canvas.getByRole('button', { name: 'Previous message page' })
    await expect(previous).toHaveAttribute('aria-disabled', 'true')
    await expect(previous).toHaveAccessibleDescription('Previous unavailable: first page.')
    await expect(canvasElement.querySelectorAll('[data-message-window]')).toHaveLength(1)
    await expect(canvasElement.querySelector('[data-message-window]')?.textContent).toBe(multiMegabyteMessage.slice(0, 16000))
    await userEvent.click(next)
    await expect(next).toHaveFocus()
    await expect(canvasElement.querySelectorAll('[data-message-window]')).toHaveLength(1)
    await expect(canvasElement.querySelector('[data-message-window]')?.textContent).toBe(multiMegabyteMessage.slice(16000, 32000))
    await expect(previous).not.toHaveAttribute('aria-disabled', 'true')
    await userEvent.click(previous)
    await expect(previous).toHaveFocus()
    await expect(previous).toHaveAttribute('aria-disabled', 'true')
    await expect(canvasElement.querySelector('[data-message-window]')?.textContent).not.toBe(multiMegabyteMessage)
  },
}
export const PagedMultiMegabyteMessageLight440: Story = { ...PagedMultiMegabyteMessage, parameters: { scheme: 'light', width: 440 } }
export const PagedMultiMegabyteMessageDark1440: Story = { ...PagedMultiMegabyteMessage, parameters: { scheme: 'dark', width: 1440 } }
export const PagedMultiMegabyteMessageLight1440: Story = { ...PagedMultiMegabyteMessage, parameters: { scheme: 'light', width: 1440 } }
export const LargeSingleLineOutgoing: Story = { render: () => <OutgoingMessageRow {...outgoingMessage} message={'Single line report '.repeat(180000)} />, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const trigger = canvas.getByRole('button', { name: /Review.*Single line report/ })
  await expect(trigger.textContent!.length).toBeLessThan(240)
  await expect(trigger.querySelector('[title]')?.getAttribute('title')?.length).toBeLessThanOrEqual(200)
  await expect(canvasElement.querySelector('[data-message-window]')).toBeNull()
  await userEvent.click(trigger)
  await expect(trigger).toHaveFocus()
  await expect(canvasElement.querySelectorAll('[data-message-window]')).toHaveLength(1)
  await expect(canvasElement.querySelector('[data-message-window]')?.textContent?.length).toBe(16000)
} }
export const LargeSingleLineOutgoingLight440: Story = { ...LargeSingleLineOutgoing, parameters: { scheme: 'light', width: 440 } }
export const LargeSingleLineOutgoingDark1440: Story = { ...LargeSingleLineOutgoing, parameters: { scheme: 'dark', width: 1440 } }
export const LargeSingleLineOutgoingLight1440: Story = { ...LargeSingleLineOutgoing, parameters: { scheme: 'light', width: 1440 } }
export const LastMessagePage: Story = { render: () => <OutgoingMessageRow {...outgoingMessage} message={'Boundary report '.repeat(1100)} defaultExpanded />, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const next = canvas.getByRole('button', { name: 'Next message page' })
  await userEvent.click(next)
  await expect(next).toHaveFocus()
  await expect(next).toHaveAttribute('aria-disabled', 'true')
  await expect(next).toHaveAccessibleDescription('Next unavailable: last page.')
  const lastWindow = canvasElement.querySelector('[data-message-window]')?.textContent
  await userEvent.click(next)
  await expect(next).toHaveFocus()
  await expect(canvasElement.querySelector('[data-message-window]')?.textContent).toBe(lastWindow)
  await expect(canvasElement.querySelectorAll('[data-message-window]')).toHaveLength(1)
  await userEvent.click(canvas.getByRole('button', { name: 'Previous message page' }))
  await expect(next).not.toHaveAttribute('aria-disabled', 'true')
} }
export const LastMessagePageLight440: Story = { ...LastMessagePage, parameters: { scheme: 'light', width: 440 } }
export const LastMessagePageDark1440: Story = { ...LastMessagePage, parameters: { scheme: 'dark', width: 1440 } }
export const LastMessagePageLight1440: Story = { ...LastMessagePage, parameters: { scheme: 'light', width: 1440 } }
export const BroadcastWithoutRecipientFacts: Story = { render: () => <OutgoingMessageRow {...broadcastMessage} recipients={[]} delivery={{ _tag: 'Broadcast' }} />, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByRole('button', { name: /Broadcast.*Please review/ })).toHaveAttribute('aria-expanded', 'false')
  await expect(canvas.queryByText(/seats/)).toBeNull()
} }
export const BroadcastWithoutRecipientFactsLight440: Story = { ...BroadcastWithoutRecipientFacts, parameters: { scheme: 'light', width: 440 } }
export const BroadcastWithoutRecipientFactsDark1440: Story = { ...BroadcastWithoutRecipientFacts, parameters: { scheme: 'dark', width: 1440 } }
export const BroadcastWithoutRecipientFactsLight1440: Story = { ...BroadcastWithoutRecipientFacts, parameters: { scheme: 'light', width: 1440 } }
const shellSend: ToolCallItem = { _tag: 'ToolCall', id: 'shell-send', callId: 'shell-send', name: 'bash', input: { command: 'message.send --to Review --body release-report' }, callSeen: true, status: 'success', at: '2026-10-11T15:40:00Z', result: { content: 'Delivered', at: '2026-10-11T15:40:01Z' } }
const shellWork = workLogTurnFromItems([shellSend], { kindFor: () => 'run', running: false, failed: false, interrupted: false, completeHistory: true })
export const ShellSendInTranscript: Story = { render: () => <EmbraceRuntimeProvider options={{ messages: [quotedPrompt, shellSend], isRunning: false, onNew: fn() }}>
  <Transcript title="Release review" turns={[{ id: 'send-turn', prompt: quotedPrompt, items: [shellSend], work: shellWork, outgoingMessages: { 'shell-send': outgoingMessage } }]} sync={{ _tag: 'Live', since: 0 }} now={0} observedAt={0} onOpenTool={fn()} />
</EmbraceRuntimeProvider>, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const fold = await canvas.findByRole('button', { name: /Worked.*1 message sent/ })
  await expect(canvas.queryByRole('button', { name: /Review.*Please review the release report/ })).toBeNull()
  await expect(fold).not.toHaveTextContent('1 command')
  await userEvent.click(fold)
  const message = canvas.getByRole('button', { name: /Review.*Please review the release report/ })
  await expect(message.closest('[data-work-kind="messages-sent"]')).toHaveAttribute('data-harness-weight', 'card')
  const work = canvas.getByTestId('work-log')
  await expect(within(work).getAllByTestId('work-log-outgoing-message')).toHaveLength(1)
  await expect(work.querySelectorAll('[data-tool-status]')).toHaveLength(0)
  await userEvent.click(message)
  await expect(canvas.getByText('The patch keeps the existing protocol and updates the examples.')).toBeVisible()
  await userEvent.click(fold)
  await expect(canvas.queryByText('The patch keeps the existing protocol and updates the examples.')).toBeNull()
} }
export const ShellSendInTranscriptLight440: Story = { ...ShellSendInTranscript, parameters: { scheme: 'light', width: 440 } }
export const ShellSendInTranscriptDark1440: Story = { ...ShellSendInTranscript, parameters: { scheme: 'dark', width: 1440 } }
export const ShellSendInTranscriptLight1440: Story = { ...ShellSendInTranscript, parameters: { scheme: 'light', width: 1440 } }
const observedCommand: ToolCallItem = { ...shellSend, id: 'observed-command', callId: 'observed-command', input: { command: 'print-report' }, result: { content: 'Report printed', at: '2026-10-11T15:40:01Z' } }
const orphanSend: ToolCallItem = { ...shellSend, id: 'orphan-send', callId: 'orphan-send', callSeen: false }
const mixedWork = workLogTurnFromItems([observedCommand, orphanSend], { kindFor: () => 'run', running: false, failed: false, interrupted: false, completeHistory: true })
export const ObservedCommandAndOrphanSend: Story = { render: () => <EmbraceRuntimeProvider options={{ messages: [quotedPrompt, observedCommand, orphanSend], isRunning: false, onNew: fn() }}>
  <Transcript title="Mixed work" turns={[{ id: 'mixed-turn', prompt: quotedPrompt, items: [observedCommand, orphanSend], work: mixedWork, outgoingMessages: { 'orphan-send': outgoingMessage } }]} sync={{ _tag: 'Live', since: 0 }} now={0} observedAt={0} onOpenTool={fn()} />
</EmbraceRuntimeProvider>, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const fold = await canvas.findByRole('button', { name: /Worked.*1 command.*1 message sent/ })
  await expect(fold).not.toHaveTextContent('2 commands')
  await userEvent.click(fold)
  const work = canvas.getByTestId('work-log')
  await expect(work.querySelectorAll('[data-tool-status]')).toHaveLength(1)
  await expect(within(work).getAllByTestId('work-log-outgoing-message')).toHaveLength(1)
  await expect(within(work).getByTestId('work-log-outgoing-message')).toHaveAttribute('data-item-id', 'orphan-send')
} }
export const ObservedCommandAndOrphanSendLight440: Story = { ...ObservedCommandAndOrphanSend, parameters: { scheme: 'light', width: 440 } }
export const ObservedCommandAndOrphanSendDark1440: Story = { ...ObservedCommandAndOrphanSend, parameters: { scheme: 'dark', width: 1440 } }
export const ObservedCommandAndOrphanSendLight1440: Story = { ...ObservedCommandAndOrphanSend, parameters: { scheme: 'light', width: 1440 } }
const styles = stylex.create({ root: { minHeight: '100vh', boxSizing: 'border-box', padding: s.xl, display: 'flex', flexDirection: 'column', gap: s.xl, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading }, narrow: { width: g.specimenNarrow, maxWidth: '100%' }, wide: { width: g.specimenWide, maxWidth: '100%' } })
