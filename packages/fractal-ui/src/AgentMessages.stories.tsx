import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import axe from 'axe-core'
import { AgentMessage, OutgoingMessageRow, type AgentMessageProps } from './assistant-ui/AgentMessage'
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

const assertMessageRhythm = async (message: HTMLElement) => {
  const header = message.querySelector<HTMLElement>('[data-message-header]')!
  const subject = message.querySelector<HTMLElement>('[data-message-subject]')!
  const body = message.querySelector<HTMLElement>('[data-message-content]')!
  await expect(message.querySelector('footer, [data-testid="answer-meta"]')).toBeNull()
  await expect(header.getBoundingClientRect().height).toBe(16)
  await expect(parseFloat(getComputedStyle(header).fontSize)).toBe(12)
  await expect(parseFloat(getComputedStyle(header).lineHeight)).toBe(16)
  await expect(parseFloat(getComputedStyle(subject).fontSize)).toBe(14)
  await expect(parseFloat(getComputedStyle(subject).lineHeight)).toBe(20)
  await expect(getComputedStyle(subject).fontWeight).toBe('500')
  await expect(subject.getBoundingClientRect().top - header.getBoundingClientRect().bottom).toBe(4)
  await expect(body.getBoundingClientRect().top - subject.getBoundingClientRect().bottom).toBe(4)
  const time = header.querySelector('time')!
  await expect(header.getBoundingClientRect().right - time.getBoundingClientRect().right).toBeLessThanOrEqual(0.5)
  await expect(message.querySelectorAll('time')).toHaveLength(1)
}

const rejectOldMessageRhythm = async (message: HTMLElement) => {
  await assertMessageRhythm(message)
  const footer = document.createElement('footer')
  footer.style.height = '1.5rem'
  footer.textContent = 'Copy · time'
  message.append(footer)
  try { await expect(assertMessageRhythm(message)).rejects.toThrow() } finally { footer.remove() }
  const subject = message.querySelector<HTMLElement>('[data-message-subject]')!
  subject.style.marginTop = '0.375rem'
  try { await expect(assertMessageRhythm(message)).rejects.toThrow() } finally { subject.style.marginTop = '' }
  const body = message.querySelector<HTMLElement>('[data-message-content]')!
  body.style.marginTop = '0.375rem'
  try { await expect(assertMessageRhythm(message)).rejects.toThrow() } finally { body.style.marginTop = '' }
  await assertMessageRhythm(message)
}

const meta = { title: 'Fractal UI/Agent Messages', component: AgentMessage, excludeStories: ['scenarioExemptions'], parameters: { scenario: { _tag: 'contract-fixture', reason: 'Exact local message bodies, delivery facts, paging boundaries and load outcomes isolate the AgentMessage and OutgoingMessageRow contracts; no scenario slice models semantic agent messages.' }, layout: 'fullscreen' }, args: { ...agentMessage, onCopyId: fn(), onLoadFull: fn(), onOpenReply: fn() }, argTypes: { scheme: { control: 'select', options: ['dark', 'light'] }, width: { control: 'select', options: [440, 1440] } }, decorators: [(Story, context) => <main data-scheme={context.args.scheme ?? context.parameters.scheme ?? 'dark'} {...stylex.props(styles.root, ...baselineTheme, (context.args.scheme ?? context.parameters.scheme) === 'light' && lightTheme, (context.args.width ?? context.parameters.width) === 1440 ? styles.wide : styles.narrow)}><Story /></main>] } satisfies Meta<AgentMessageProps & { readonly scheme?: 'dark' | 'light'; readonly width?: 440 | 1440 }>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStates:contract-fixture',
  'AllStatesDark1440:contract-fixture',
  'AllStatesLight1440:contract-fixture',
  'AllStatesLight440:contract-fixture',
  'BroadcastCopy:contract-fixture',
  'BroadcastWithoutCount:contract-fixture',
  'BroadcastWithoutCountDark1440:contract-fixture',
  'BroadcastWithoutCountLight1440:contract-fixture',
  'BroadcastWithoutCountLight440:contract-fixture',
  'BroadcastWithoutRecipientFacts:contract-fixture',
  'BroadcastWithoutRecipientFactsDark1440:contract-fixture',
  'BroadcastWithoutRecipientFactsLight1440:contract-fixture',
  'BroadcastWithoutRecipientFactsLight440:contract-fixture',
  'Clipped:contract-fixture',
  'ClippedDark1440:contract-fixture',
  'ClippedLight1440:contract-fixture',
  'ClippedLight440:contract-fixture',
  'CompositionFrames:contract-fixture',
  'CompositionFramesDark1440:contract-fixture',
  'CompositionFramesLight1440:contract-fixture',
  'CompositionFramesLight440:contract-fixture',
  'FailedMessageTransition:contract-fixture',
  'FetchedLongMessage:contract-fixture',
  'FetchedLongMessageDark1440:contract-fixture',
  'FetchedLongMessageLight1440:contract-fixture',
  'FetchedLongMessageLight440:contract-fixture',
  'FromParent:contract-fixture',
  'FullMessageTransition:contract-fixture',
  'FullMessageTransitionDark1440:contract-fixture',
  'FullMessageTransitionLight1440:contract-fixture',
  'FullMessageTransitionLight440:contract-fixture',
  'HierarchyHuman:contract-fixture',
  'HierarchyHumanCard:contract-fixture',
  'HierarchyHumanCardDark1440:contract-fixture',
  'HierarchyHumanCardLight1440:contract-fixture',
  'HierarchyHumanCardLight440:contract-fixture',
  'HierarchyHumanDark1440:contract-fixture',
  'HierarchyHumanLight1440:contract-fixture',
  'HierarchyHumanLight440:contract-fixture',
  'HierarchyIncoming:contract-fixture',
  'HierarchyIncomingDark1440:contract-fixture',
  'HierarchyIncomingLight1440:contract-fixture',
  'HierarchyIncomingLight440:contract-fixture',
  'HierarchyOutgoing:contract-fixture',
  'HierarchyOutgoingDark1440:contract-fixture',
  'HierarchyOutgoingLight1440:contract-fixture',
  'HierarchyOutgoingLight440:contract-fixture',
  'HierarchyWithoutSubject:contract-fixture',
  'HierarchyWithoutSubjectDark1440:contract-fixture',
  'HierarchyWithoutSubjectLight1440:contract-fixture',
  'HierarchyWithoutSubjectLight440:contract-fixture',
  'InterAgent:contract-fixture',
  'LargeBodies:contract-fixture',
  'LargeBodiesDark1440:contract-fixture',
  'LargeBodiesLight1440:contract-fixture',
  'LargeBodiesLight440:contract-fixture',
  'LargeSingleLineOutgoing:contract-fixture',
  'LargeSingleLineOutgoingDark1440:contract-fixture',
  'LargeSingleLineOutgoingLight1440:contract-fixture',
  'LargeSingleLineOutgoingLight440:contract-fixture',
  'LastMessagePage:contract-fixture',
  'LastMessagePageDark1440:contract-fixture',
  'LastMessagePageLight1440:contract-fixture',
  'LastMessagePageLight440:contract-fixture',
  'LoadFailed:contract-fixture',
  'Loading:contract-fixture',
  'Long:contract-fixture',
  'LongDark1440:contract-fixture',
  'LongLight1440:contract-fixture',
  'LongLight440:contract-fixture',
  'NegativeControls:contract-fixture',
  'NegativeControlsDark1440:contract-fixture',
  'NegativeControlsLight1440:contract-fixture',
  'NegativeControlsLight440:contract-fixture',
  'ObservedCommandAndOrphanSend:contract-fixture',
  'ObservedCommandAndOrphanSendDark1440:contract-fixture',
  'ObservedCommandAndOrphanSendLight1440:contract-fixture',
  'ObservedCommandAndOrphanSendLight440:contract-fixture',
  'OutgoingBroadcast:contract-fixture',
  'OutgoingBroadcastDark1440:contract-fixture',
  'OutgoingBroadcastLight1440:contract-fixture',
  'OutgoingBroadcastLight440:contract-fixture',
  'OutgoingFailed:contract-fixture',
  'OutgoingFailedDark1440:contract-fixture',
  'OutgoingFailedLight1440:contract-fixture',
  'OutgoingFailedLight440:contract-fixture',
  'OutgoingSingle:contract-fixture',
  'OutgoingSingleDark1440:contract-fixture',
  'OutgoingSingleLight1440:contract-fixture',
  'OutgoingSingleLight440:contract-fixture',
  'OutgoingWithoutReceiptFacts:contract-fixture',
  'PagedMultiMegabyteMessage:contract-fixture',
  'PagedMultiMegabyteMessageDark1440:contract-fixture',
  'PagedMultiMegabyteMessageLight1440:contract-fixture',
  'PagedMultiMegabyteMessageLight440:contract-fixture',
  'Reply:contract-fixture',
  'ReplyDark1440:contract-fixture',
  'ReplyLight1440:contract-fixture',
  'ReplyLight440:contract-fixture',
  'ReplyMarker:contract-fixture',
  'ReplyMarkerDark1440:contract-fixture',
  'ReplyMarkerLight1440:contract-fixture',
  'ReplyMarkerLight440:contract-fixture',
  'ShellSendInTranscript:contract-fixture',
  'ShellSendInTranscriptDark1440:contract-fixture',
  'ShellSendInTranscriptLight1440:contract-fixture',
  'ShellSendInTranscriptLight440:contract-fixture',
  'Short:contract-fixture',
  'ShortDark1440:contract-fixture',
  'ShortLight1440:contract-fixture',
  'ShortLight440:contract-fixture',
  'SmallTalk:contract-fixture',
  'Teammate:contract-fixture',
  'UnavailableFullMessage:contract-fixture',
  'Untitled:contract-fixture',
]
type Story = StoryObj<typeof meta>
/** Every visible word and glyph of a message header sits inside its row box; visually hidden copies are skipped. */
const assertHeaderInsideRow = async (article: HTMLElement) => {
  const header = article.querySelector<HTMLElement>('[data-message-header]')!
  const row = header.getBoundingClientRect()
  const hidden = (element: Element | null): boolean => element !== null && element !== header && (getComputedStyle(element).clipPath !== 'none' || hidden(element.parentElement))
  const outside: string[] = []
  const walker = document.createTreeWalker(header, NodeFilter.SHOW_TEXT)
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    if (node.textContent!.trim() === '' || hidden(node.parentElement)) continue
    const range = document.createRange()
    range.selectNodeContents(node)
    for (const rect of range.getClientRects()) if (rect.top < row.top - 1 || rect.bottom > row.bottom + 1) outside.push(`${node.textContent} ${Math.round(rect.top)}..${Math.round(rect.bottom)}`)
  }
  for (const glyph of header.querySelectorAll('svg')) {
    const rect = glyph.getBoundingClientRect()
    if (!hidden(glyph) && (rect.top < row.top - 1 || rect.bottom > row.bottom + 1)) outside.push(`svg ${glyph.getAttribute('data-icon')} ${Math.round(rect.top)}..${Math.round(rect.bottom)}`)
  }
  await expect(outside).toEqual([])
}
/** The header check rejects a counterpart that wraps onto a second line, then the restored header passes again. */
const assertHeaderInsideRowWithControl = async (article: HTMLElement) => {
  await assertHeaderInsideRow(article)
  const wrap = document.createElement('span')
  wrap.style.display = 'block'
  wrap.textContent = 'Injected'
  article.querySelector('[data-message-header] strong')!.append(wrap)
  try { await expect(assertHeaderInsideRow(article)).rejects.toThrow() } finally { wrap.remove() }
  await assertHeaderInsideRow(article)
}
export const Short: Story = { play: async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByRole('article', { name: 'Message from Parent.Review' })).toHaveAttribute('data-harness-weight', 'card')
  // "From Parent › Review → Parent › Build · IRC" stays on the 16-high header row at every width.
  await assertHeaderInsideRowWithControl(canvas.getByRole('article', { name: 'Message from Parent.Review' }))
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
  // With onOpenReply the visible button is the reply path; no icon-only marker and no native title.
  await expect(canvas.queryByRole('img', { name: 'Reply to Parent.Question' })).toBeNull()
  await expect(canvasElement.querySelectorAll('[title]')).toHaveLength(0)
} }
const openTooltips = () => [...document.querySelectorAll<HTMLElement>('[role="tooltip"]')].map(tooltip => tooltip.textContent)
/** Distinct rendered text lines of an element. */
const textLineCount = (element: HTMLElement) => {
  const range = document.createRange()
  range.selectNodeContents(element)
  return new Set([...range.getClientRects()].filter(rect => rect.width > 0).map(rect => Math.round(rect.top))).size
}
/** The open card is readable: its short label renders on one line, not one character per line. A squeezed card is rejected. */
const assertReadableCard = async () => {
  const card = await waitFor(() => {
    const candidate = document.querySelector<HTMLElement>('[role="tooltip"]')
    expect(candidate).toBeInstanceOf(Node)
    expect(textLineCount(candidate!)).toBe(1)
    return candidate!
  }, { timeout: 3_000 })
  card.style.width = '0px'
  try { await expect(textLineCount(card)).toBeGreaterThan(1) } finally { card.style.removeProperty('width') }
  await expect(textLineCount(card)).toBe(1)
}
/**
 * The kit hover card is the visible path for a fact that used to be a native title: exactly one readable card on keyboard
 * focus and on hover, and the focused trigger draws the kit focus ring. React Aria opens focus cards only in keyboard modality
 * and hover cards only in pointer modality, so reach the target with real Tab presses first, then switch to pointer modality
 * with a real click on the page before hovering.
 */
const assertKitTooltip = async (target: HTMLElement, label: string) => {
  for (let step = 0; step < 12 && target.ownerDocument.activeElement !== target; step++) await userEvent.tab()
  await expect(target).toHaveFocus()
  await expect(getComputedStyle(target).outlineStyle).toBe('solid')
  await expect(parseFloat(getComputedStyle(target).outlineWidth)).toBeGreaterThan(0)
  await waitFor(() => expect(openTooltips()).toEqual([label]), { timeout: 3_000 })
  await assertReadableCard()
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(openTooltips()).toEqual([]), { timeout: 3_000 })
  await userEvent.click(target.ownerDocument.body)
  await expect(target).not.toHaveFocus()
  await expect(getComputedStyle(target).outlineStyle).not.toBe('solid')
  await userEvent.hover(target)
  await waitFor(() => expect(openTooltips()).toEqual([label]), { timeout: 3_000 })
  await assertReadableCard()
  await userEvent.unhover(target)
  await waitFor(() => expect(openTooltips()).toEqual([]), { timeout: 3_000 })
}
export const ReplyMarker: Story = { args: { replyTo: 'Parent.Question', subject: 'Release review', onOpenReply: undefined }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const marker = canvas.getByRole('img', { name: 'Reply to Parent.Question' })
  await expect(canvas.queryByRole('button', { name: /Reply to/ })).toBeNull()
  const noTitles = async () => expect(canvasElement.querySelectorAll('[title]')).toHaveLength(0)
  await noTitles()
  marker.setAttribute('title', 'injected')
  try { await expect(noTitles()).rejects.toThrow() } finally { marker.removeAttribute('title') }
  await noTitles()
  await assertKitTooltip(marker, 'Reply to Parent.Question')
  await noTitles()
  await assertHeaderInsideRowWithControl(canvasElement.querySelector<HTMLElement>('[data-message-variant]')!)
} }
export const ReplyLight440: Story = { ...Reply, parameters: { scheme: 'light', width: 440 } }
export const ReplyDark1440: Story = { ...Reply, parameters: { scheme: 'dark', width: 1440 } }
export const ReplyLight1440: Story = { ...Reply, parameters: { scheme: 'light', width: 1440 } }
export const ReplyMarkerLight440: Story = { ...ReplyMarker, parameters: { scheme: 'light', width: 440 } }
export const ReplyMarkerDark1440: Story = { ...ReplyMarker, parameters: { scheme: 'dark', width: 1440 } }
export const ReplyMarkerLight1440: Story = { ...ReplyMarker, parameters: { scheme: 'light', width: 1440 } }
export const FromParent: Story = { args: { from: 'Parent', to: 'Parent.Child', via: { _tag: 'FromParent' } }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByText('from parent')).toBeVisible()
  const hierarchy = canvasElement.querySelector('[data-agent-id="Parent.Child"]')!
  await expect(hierarchy.textContent).toContain('ParentChild')
  await expect(hierarchy.querySelector('svg[data-icon="chevron-right"]')).not.toBeNull()
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
  await expect(canvasElement.querySelectorAll('[data-harness-weight="card"]')).toHaveLength(12)
  await expect(canvasElement.querySelectorAll('[data-harness-weight="row"]')).toHaveLength(3)
  await expect(canvas.queryByText('End of the full release review.')).toBeNull()
  await expect(canvas.getByRole('alert')).toHaveTextContent('the preview is kept')
  const subject = canvasElement.querySelector<HTMLElement>('[data-message-subject]')!
  const assertHeading = async () => { await expect(canvas.getByRole('heading', { name: 'Release review', level: 2 })).toBeVisible() }
  await assertHeading()
  const skippedHeading = document.createElement('h3')
  skippedHeading.textContent = subject.textContent
  subject.replaceWith(skippedHeading)
  try { await expect(assertHeading()).rejects.toThrow() } finally { skippedHeading.replaceWith(subject) }
  await assertHeading()
} }
export const OutgoingSingle: Story = { render: () => <OutgoingMessageRow {...outgoingMessage} />, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const trigger = canvas.getByRole('button', { name: /Review.*Please review the release report/ })
  await expect(trigger).toHaveAttribute('aria-expanded', 'false')
  // No native titles: the full first line is the exact name tail and the send time the exact description.
  await expect(canvasElement.querySelectorAll('[title]')).toHaveLength(0)
  await expect(trigger).toHaveAccessibleName('To Review · Please review the release report.')
  await expect(trigger).toHaveAccessibleDescription(outgoingMessage.at!)
  await expect(canvasElement.contains(document.getElementById(trigger.getAttribute('aria-describedby')!))).toBe(true)
  // The send time stays visible: the kit hover card on hover and on keyboard focus, with the same text as the description.
  await assertKitTooltip(trigger, outgoingMessage.at!)
  await expect(trigger).toHaveAccessibleDescription(outgoingMessage.at!)
  await expect(canvas.queryByText('The patch keeps the existing protocol and updates the examples.')).toBeNull()
  await userEvent.click(trigger)
  await expect(trigger).toHaveAttribute('aria-expanded', 'true')
  await expect(canvas.getByText('The patch keeps the existing protocol and updates the examples.')).toBeVisible()
  await expect(canvasElement.querySelectorAll('[title]')).toHaveLength(0)
  await expect(trigger).toHaveFocus()
  await userEvent.keyboard('{Enter}')
  await expect(trigger).toHaveAttribute('aria-expanded', 'false')
  await expect(trigger).toHaveFocus()
  await expect(canvas.queryByText('The patch keeps the existing protocol and updates the examples.')).toBeNull()
} }
export const OutgoingBroadcast: Story = { render: () => <OutgoingMessageRow {...broadcastMessage} />, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const trigger = canvas.getByRole('button', { name: /3 seats/ })
  await expect(trigger).toHaveAttribute('aria-expanded', 'false')
  const assertFolded = async () => {
    await expect(canvas.getByText('3 receipts')).toBeVisible()
    await expect(canvas.queryByText(/injected|queued|revived/)).toBeNull()
  }
  await assertFolded()
  const control = document.createElement('span')
  control.textContent = 'injected'
  canvasElement.append(control)
  try { await expect(assertFolded()).rejects.toThrow() } finally { control.remove() }
  await userEvent.click(trigger)
  await expect(canvas.getByText(/injected/)).toBeVisible()
  await expect(canvas.getByText(/queued/)).toBeVisible()
  await expect(canvas.getByText(/revived/)).toBeVisible()
  await expect(canvas.queryByText(/failed/)).toBeNull()
} }
export const OutgoingFailed: Story = { render: () => <OutgoingMessageRow {...outgoingMessage} recipients={[{ id: 'Review', receipt: { _tag: 'Failed', reason: 'Recipient retired before delivery' } }]} />, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.queryByText('Review: Recipient retired before delivery')).toBeNull()
  await userEvent.click(canvas.getByRole('button', { name: /Please review the release report/ }))
  const receipt = canvasElement.querySelector<HTMLElement>('[aria-label="Review · failed"]')!
  receipt.focus()
  await expect(receipt).toHaveFocus()
  // Named once, described once: the reason is not repeated in the name.
  await expect(receipt).toHaveAccessibleName('Review · failed')
  // The failure reason is the receipt's exact description, not a native title.
  const noTitles = async () => expect(canvasElement.querySelectorAll('[title]')).toHaveLength(0)
  await noTitles()
  await expect(receipt).toHaveAccessibleDescription('Recipient retired before delivery')
  receipt.setAttribute('title', 'injected')
  try { await expect(noTitles()).rejects.toThrow() } finally { receipt.removeAttribute('title') }
  await noTitles()
  await expect(canvas.getByText('Review: Recipient retired before delivery')).toBeVisible()
  await expect(canvas.queryByText(/injected/)).toBeNull()
} }
const quotedPrompt: TextItem & { role: 'user' } = { _tag: 'Text', role: 'user', id: 'quoted-message', text: 'Explain this example: <smalltalk-message from="Review">Ready for review.</smalltalk-message>', attachments: [], streaming: false, at: '2026-10-11T15:40:00Z' }
const quotedWork = workLogTurnFromItems([quotedPrompt], { kindFor: () => 'read', running: false, failed: false, interrupted: false, completeHistory: true })
export const NegativeControls: Story = { render: (args, context) => <>
  <EmbraceRuntimeProvider options={{ messages: [quotedPrompt], isRunning: false, onNew: fn() }}><Transcript title="Quoted example" turns={[{ id: 'quote-turn', prompt: quotedPrompt, items: [], work: quotedWork }]} sync={{ _tag: 'Live', since: 0 }} now={0} observedAt={0} /></EmbraceRuntimeProvider>
  <div data-testid="legacy-file-control" {...stylex.props((args.scheme ?? context.parameters.scheme) !== 'light' && legacyToolDarkTheme)}><EmbraceToolItem item={{ _tag: 'ToolCall', id: 'file-write', callId: 'file-write', name: 'write', input: { path: 'notes/agent-plan.md', content: 'Keep the release plan beside the report.' }, status: 'success', callSeen: true, at: '2026-10-11T15:40:00Z' }} variant="cards" /></div>
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
  // The excerpt is bounded in the name itself (no native title carries it): 'To Review · ' plus at most 200 characters.
  await expect(trigger.querySelectorAll('[title]')).toHaveLength(0)
  await expect(trigger.getAttribute('aria-label')!.length).toBeLessThanOrEqual('To Review · '.length + 200)
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
const shellSend: ToolCallItem = { _tag: 'ToolCall', id: 'shell-send', callId: 'shell-send', name: 'bash', input: { command: 'message.send --to Review --body release-report' }, callSeen: true, status: 'success', at: '2026-10-11T15:40:00Z', result: { content: 'Delivered', at: '2026-10-11T15:40:01Z', isError: false } }
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
  await expect(message.closest('[data-work-kind="messages-sent"]')).toHaveAttribute('data-harness-weight', 'row')
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
const observedCommand: ToolCallItem = { ...shellSend, id: 'observed-command', callId: 'observed-command', input: { command: 'print-report' }, result: { content: 'Report printed', at: '2026-10-11T15:40:01Z', isError: false } }
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
  // Production composition: Tab scrolls the send row into view inside the transcript; the card must survive that scroll.
  await assertKitTooltip(within(work).getByRole('button', { name: /Please review the release report/ }), outgoingMessage.at!)
} }
export const ObservedCommandAndOrphanSendLight440: Story = { ...ObservedCommandAndOrphanSend, parameters: { scheme: 'light', width: 440 } }
export const ObservedCommandAndOrphanSendDark1440: Story = { ...ObservedCommandAndOrphanSend, parameters: { scheme: 'dark', width: 1440 } }
export const ObservedCommandAndOrphanSendLight1440: Story = { ...ObservedCommandAndOrphanSend, parameters: { scheme: 'light', width: 1440 } }
const styles = stylex.create({ root: { minHeight: '100vh', boxSizing: 'border-box', padding: s.xl, display: 'flex', flexDirection: 'column', gap: s.xl, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading }, narrow: { width: g.specimenNarrow, maxWidth: '100%' }, wide: { width: g.specimenWide, maxWidth: '100%' } })

export const HierarchyIncoming: Story = {
  args: { variant: { _tag: 'Incoming' }, subject: 'Release report ready', onCopy: fn(), body: { _tag: 'Full', markdown: 'The reviewers have a report to inspect.\n\n- Review the release\n  - Check keyboard focus\n  - Compare both schemes\n- Publish after approval' } },
  play: async ({ canvasElement, args }) => {
    const message = canvasElement.querySelector<HTMLElement>('[data-message-variant]')!
    await rejectOldMessageRhythm(message)
    const header = message.querySelector<HTMLElement>('[data-message-header]')!
    const glyph = header.querySelector<HTMLElement>('[data-message-glyph]')!
    const assertGlyphContained = async () => {
      const bounds = glyph.getBoundingClientRect()
      const line = header.getBoundingClientRect()
      await expect(header.querySelectorAll('[data-message-glyph]')).toHaveLength(1)
      await expect(glyph.querySelector('svg[data-icon]')).not.toBeNull()
      await expect(glyph.textContent).toBe('')
      await expect(bounds.width).toBe(16)
      await expect(bounds.height).toBe(16)
      await expect(bounds.left).toBeGreaterThanOrEqual(line.left)
      await expect(bounds.right).toBeLessThanOrEqual(line.right)
      await expect(bounds.top).toBeGreaterThanOrEqual(line.top)
      await expect(bounds.bottom).toBeLessThanOrEqual(line.bottom)
    }
    await assertGlyphContained()
    glyph.style.width = '1.5rem'
    glyph.style.height = '1.5rem'
    try { await expect(assertGlyphContained()).rejects.toThrow() } finally { glyph.style.width = ''; glyph.style.height = '' }
    const glyphSvg = glyph.querySelector('svg')!
    glyphSvg.remove()
    glyph.textContent = 'R'
    try { await expect(assertGlyphContained()).rejects.toThrow() } finally { glyph.textContent = ''; glyph.append(glyphSvg) }
    await assertGlyphContained()
    const copy = within(header).getByRole('button', { name: 'Copy message' })
    await expect(message.querySelectorAll('button')).toHaveLength(1)
    await expect(message.querySelector('ul ul')).not.toBeNull()
    const markdown = message.querySelector<HTMLElement>('[data-testid="markdown"]')!
    const outerList = markdown.querySelector<HTMLElement>('ul')!
    const nestedList = markdown.querySelector<HTMLElement>('ul ul')!
    const firstBlock = markdown.firstElementChild as HTMLElement
    const lastBlock = markdown.lastElementChild as HTMLElement
    const assertMarkdown = async () => {
      await expect(parseFloat(getComputedStyle(markdown).gap)).toBe(8)
      await expect(parseFloat(getComputedStyle(firstBlock).marginTop)).toBe(0)
      await expect(parseFloat(getComputedStyle(lastBlock).marginBottom)).toBe(0)
      await expect(parseFloat(getComputedStyle(outerList).paddingInlineStart)).toBe(22)
      await expect(parseFloat(getComputedStyle(nestedList).paddingInlineStart)).toBe(20)
      await expect(parseFloat(getComputedStyle(nestedList).borderInlineStartWidth)).toBe(1)
      await expect(getComputedStyle(nestedList).listStyleType).toBe('circle')
      await expect(getComputedStyle(firstBlock).color).toBe(getComputedStyle(message).color)
    }
    await assertMarkdown()
    markdown.style.gap = '0.625rem'
    try { await expect(assertMarkdown()).rejects.toThrow() } finally { markdown.style.gap = '' }
    lastBlock.style.marginBottom = '0.625rem'
    try { await expect(assertMarkdown()).rejects.toThrow() } finally { lastBlock.style.marginBottom = '' }
    nestedList.style.borderInlineStartWidth = '0'
    try { await expect(assertMarkdown()).rejects.toThrow() } finally { nestedList.style.borderInlineStartWidth = '' }
    firstBlock.style.color = 'rgb(120, 120, 120)'
    try { await expect(assertMarkdown()).rejects.toThrow() } finally { firstBlock.style.color = '' }
    await assertMarkdown()
    if (args.variant?._tag === 'Incoming' || (args.variant?._tag === 'Human' && args.variant.frame?._tag === 'Card')) {
      const assertPadding = async () => {
        const computed = getComputedStyle(message)
        await expect(parseFloat(computed.paddingTop)).toBe(8)
        await expect(parseFloat(computed.paddingBottom)).toBe(8)
        await expect(parseFloat(computed.paddingLeft)).toBe(12)
        await expect(parseFloat(computed.paddingRight)).toBe(12)
      }
      await assertPadding()
      message.style.paddingBlock = '0.75rem'
      try { await expect(assertPadding()).rejects.toThrow() } finally { message.style.paddingBlock = '' }
      await assertPadding()
      await expect(parseFloat(getComputedStyle(message).borderTopWidth)).toBe(1)
      await expect(parseFloat(getComputedStyle(message).borderRadius)).toBe(8)
      await expect(parseContrastColor(getComputedStyle(message).backgroundColor)[3]).toBe(1)
    } else {
      const assertHumanRail = async () => expect(parseFloat(getComputedStyle(message).borderLeftWidth)).toBe(2)
      await assertHumanRail()
      message.style.borderLeftWidth = '0'
      try { await expect(assertHumanRail()).rejects.toThrow() } finally { message.style.borderLeftWidth = '' }
      await assertHumanRail()
      await expect(parseContrastColor(getComputedStyle(message).backgroundColor)[3]).toBe(0)
    }
    message.tabIndex = -1
    message.focus()
    await userEvent.tab()
    await expect(copy).toHaveFocus()
    await expect(getComputedStyle(copy).opacity).toBe('1')
    await userEvent.keyboard('{Enter}')
    await expect(args.onCopy).toHaveBeenCalledTimes(1)
    message.removeAttribute('tabindex')
    const assertTabStops = async () => expect([...message.querySelectorAll('[tabindex="0"]')].filter(element => element !== copy)).toHaveLength(0)
    await assertTabStops()
    const additionalStop = document.createElement('span')
    additionalStop.tabIndex = 0
    message.append(additionalStop)
    try { await expect(assertTabStops()).rejects.toThrow() } finally { additionalStop.remove() }
    await assertMessageRhythm(message)
  },
}
export const HierarchyIncomingLight440: Story = { ...HierarchyIncoming, parameters: { scheme: 'light', width: 440 } }
export const HierarchyIncomingDark1440: Story = { ...HierarchyIncoming, parameters: { scheme: 'dark', width: 1440 } }
export const HierarchyIncomingLight1440: Story = { ...HierarchyIncoming, parameters: { scheme: 'light', width: 1440 } }
export const HierarchyHuman: Story = { ...HierarchyIncoming, args: { ...HierarchyIncoming.args, variant: { _tag: 'Human' }, from: 'Reviewer', to: undefined, via: undefined } }
export const HierarchyHumanLight440: Story = { ...HierarchyHuman, parameters: { scheme: 'light', width: 440 } }
export const HierarchyHumanDark1440: Story = { ...HierarchyHuman, parameters: { scheme: 'dark', width: 1440 } }
export const HierarchyHumanLight1440: Story = { ...HierarchyHuman, parameters: { scheme: 'light', width: 1440 } }
export const HierarchyOutgoing: Story = {
  args: { onCopy: fn() },
  render: args => <OutgoingMessageRow {...outgoingMessage} subject="Release report ready" defaultExpanded onCopy={args.onCopy} />,
  play: async ({ canvasElement, args }) => {
    const message = canvasElement.querySelector<HTMLElement>('[data-message-variant="outgoing"]')!
    const header = message.querySelector<HTMLElement>('[data-message-header]')!
    const assertWorkRow = async () => {
      await expect(header.getBoundingClientRect().height).toBe(24)
      const columns = getComputedStyle(header).gridTemplateColumns.split(' ').map(parseFloat)
      await expect(getComputedStyle(header).display).toBe('grid')
      await expect(columns.slice(-2)).toEqual([16, 80])
      const disclosure = header.querySelector<HTMLElement>('[data-message-disclosure]')!
      await expect(header.getBoundingClientRect().right - disclosure.getBoundingClientRect().left).toBe(102)
      await expect(parseFloat(getComputedStyle(message).borderLeftWidth)).toBe(0)
      await expect(header.querySelector('time')).toBeNull()
      await expect(header.querySelector('button[slot="trigger"]')?.textContent?.match(/\bReview\b/g) ?? []).toHaveLength(1)
      await expect(header.querySelector('[data-message-glyph] svg')).not.toBeNull()
      await expect(message.querySelector('[data-fractal-card]')).toBeNull()
      const opened = message.querySelector<HTMLElement>('[data-opened-body]')!
      await expect(opened.getBoundingClientRect().top - header.getBoundingClientRect().bottom).toBe(2)
      await expect(parseFloat(getComputedStyle(opened).paddingTop)).toBe(8)
      await expect(parseFloat(getComputedStyle(opened).paddingLeft)).toBe(12)
      await expect(parseFloat(getComputedStyle(opened).borderLeftWidth)).toBe(1)
    }
    await assertWorkRow()
    message.style.borderLeft = '0.125rem solid currentColor'
    try { await expect(assertWorkRow()).rejects.toThrow() } finally { message.style.borderLeft = '' }
    header.style.height = '1rem'
    try { await expect(assertWorkRow()).rejects.toThrow() } finally { header.style.height = '' }
    header.style.display = 'flex'
    try { await expect(assertWorkRow()).rejects.toThrow() } finally { header.style.display = '' }
    const trigger = within(header).getByRole('button', { name: /To Review/ })
    const duplicateId = document.createElement('span')
    duplicateId.textContent = 'Review'
    trigger.append(duplicateId)
    try { await expect(assertWorkRow()).rejects.toThrow() } finally { duplicateId.remove() }
    const copy = within(header).getByRole('button', { name: 'Copy message' })
    trigger.focus()
    await userEvent.tab()
    await expect(copy).toHaveFocus()
    await userEvent.keyboard('{Enter}')
    await expect(args.onCopy).toHaveBeenCalledTimes(1)
    await expect(message.querySelectorAll('button')).toHaveLength(2)
    await expect(message.querySelector('[data-message-content]')?.textContent).not.toContain('Release report ready')
    await userEvent.click(trigger)
    await expect(trigger).toHaveAttribute('aria-expanded', 'false')
    await expect(message.querySelector('[data-message-content]')).toBeNull()
    await expect(message.querySelector('footer')).toBeNull()
    await expect(trigger).toHaveFocus()
    await userEvent.keyboard('{Enter}')
    await assertWorkRow()
  },
}
export const HierarchyOutgoingLight440: Story = { ...HierarchyOutgoing, parameters: { scheme: 'light', width: 440 } }
export const HierarchyOutgoingDark1440: Story = { ...HierarchyOutgoing, parameters: { scheme: 'dark', width: 1440 } }
export const HierarchyOutgoingLight1440: Story = { ...HierarchyOutgoing, parameters: { scheme: 'light', width: 1440 } }

export const HierarchyWithoutSubject: Story = {
  render: () => <>
    <AgentMessage {...agentMessage} />
    <OutgoingMessageRow {...outgoingMessage} defaultExpanded />
    <AgentMessage {...agentMessage} variant={{ _tag: 'Human' }} from="Reviewer" to={undefined} via={undefined} />
  </>,
  play: async ({ canvasElement }) => {
    const messages = canvasElement.querySelectorAll<HTMLElement>('[data-message-variant]')
    await expect(messages).toHaveLength(3)
    for (const message of messages) {
      const header = message.querySelector<HTMLElement>('[data-message-header]')!
      const body = message.querySelector<HTMLElement>('[data-opened-body]') ?? message.querySelector<HTMLElement>('[data-message-content]')!
      const assertGap = async () => {
        await expect(message.querySelector('[data-message-subject], footer')).toBeNull()
        await expect(body.getBoundingClientRect().top - header.getBoundingClientRect().bottom).toBe(2)
      }
      await assertGap()
      body.style.marginTop = '0.25rem'
      try { await expect(assertGap()).rejects.toThrow() } finally { body.style.marginTop = '' }
      await assertGap()
    }
  },
}
export const HierarchyWithoutSubjectLight440: Story = { ...HierarchyWithoutSubject, parameters: { scheme: 'light', width: 440 } }
export const HierarchyWithoutSubjectDark1440: Story = { ...HierarchyWithoutSubject, parameters: { scheme: 'dark', width: 1440 } }
export const HierarchyWithoutSubjectLight1440: Story = { ...HierarchyWithoutSubject, parameters: { scheme: 'light', width: 1440 } }

export const HierarchyHumanCard: Story = { ...HierarchyHuman, args: { ...HierarchyHuman.args, variant: { _tag: 'Human', frame: { _tag: 'Card' } } } }
export const HierarchyHumanCardLight440: Story = { ...HierarchyHumanCard, parameters: { scheme: 'light', width: 440 } }
export const HierarchyHumanCardDark1440: Story = { ...HierarchyHumanCard, parameters: { scheme: 'dark', width: 1440 } }
export const HierarchyHumanCardLight1440: Story = { ...HierarchyHumanCard, parameters: { scheme: 'light', width: 1440 } }

export const CompositionFrames: Story = {
  render: () => <>
    <AgentMessage {...agentMessage} />
    <AgentMessage {...agentMessage} variant={{ _tag: 'Human' }} from="Reviewer" />
    <AgentMessage {...agentMessage} variant={{ _tag: 'Human', frame: { _tag: 'Card' } }} from="Reviewer" />
    <OutgoingMessageRow {...broadcastMessage} subject="Re: Re: Release review" defaultExpanded />
  </>,
  play: async ({ canvasElement }) => {
    const assertFrames = async () => {
      await expect(canvasElement.querySelector('[data-fractal-card] [data-fractal-card]')).toBeNull()
      await expect(canvasElement.querySelectorAll('[data-fractal-card]')).toHaveLength(2)
      for (const header of canvasElement.querySelectorAll('[data-message-header]')) {
        await expect(header.querySelectorAll('[data-message-glyph] svg')).toHaveLength(1)
      }
      await expect(within(canvasElement).getByText('Re: Release review')).toBeVisible()
    }
    await assertFrames()
    const card = canvasElement.querySelector<HTMLElement>('[data-fractal-card]')!
    const nested = card.cloneNode(false) as HTMLElement
    card.append(nested)
    try { await expect(assertFrames()).rejects.toThrow() } finally { nested.remove() }
    await assertFrames()
  },
}
export const CompositionFramesLight440: Story = { ...CompositionFrames, parameters: { scheme: 'light', width: 440 } }
export const CompositionFramesDark1440: Story = { ...CompositionFrames, parameters: { scheme: 'dark', width: 1440 } }
export const CompositionFramesLight1440: Story = { ...CompositionFrames, parameters: { scheme: 'light', width: 1440 } }
