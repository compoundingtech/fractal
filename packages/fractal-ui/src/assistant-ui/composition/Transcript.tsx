import { markdownBlock } from './markdown-text'
import * as React from 'react'
import { createPortal } from 'react-dom'
import * as stylex from '@stylexjs/stylex'
import { MessagePrimitive, ThreadPrimitive, useAuiState } from '@assistant-ui/react'
import { Button, Disclosure, DisclosurePanel, ProgressBar, VisuallyHidden } from 'react-aria-components'
import type { ConversationItem, MessageItem, TextItem } from '../embrace-data/model'
import { EmbraceScrollViewport, type ViewportAnchorHistory } from '../EmbraceScrollViewport'
import { RuntimeAdoptedIds } from '../EmbraceRuntime'
import { WorkLogV1 } from '../taste/WorkLogV1'
import { errorReason, formatWorkDuration, readableCommand, workLogOutputLanguage, type WorkLogCall, type WorkLogTurn } from '../taste/work-log'
import { SyncLine } from '../sync-presentation/SyncLine'
import { syncLine } from '../sync-presentation/sync-line'
import type { SyncStatus } from '../sync-presentation/sync-status'
import { ErrorOverlay, ErrorOverlayHost } from './ErrorOverlay'
import { completeStreamingTail, HighlightedSource, Markdown, MarkdownImagePolicy, type InlineReferenceRenderer, type MarkdownImageOpener, type MarkdownImageResolver } from './Markdown'
import { ThinkingRun } from './ThinkingEntry'
import { SendStatus, TranscriptEmptyContent, type TranscriptEmptyState } from './TranscriptFeedback'
import type { ComposerAvailability } from '../embrace-composer/send'
import { TranscriptAttachmentImagesContext, TranscriptAttachments, type TranscriptAttachmentImages } from './TranscriptAttachments'
import { HarnessNotice } from './HarnessNotice'
import { MailMessage, type MailParticipants } from './MailMessage'
import { UnsupportedRecord } from './UnsupportedRecord'
import { TranscriptSkeleton } from './TranscriptSkeleton'
import { Icon } from './Icons'
import { surfaceVars as surface, textVars as ink, borderVars as border, accentVars as accent, interactionVars as interaction, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g, motionVars as m, geometryNumbers } from '../composition-tokens.stylex'
import { readingColumnStyles } from '../reading-column.stylex'
import { Tooltip } from './Controls'
import { useCopyFeedback, type CopyAction } from './useCopyFeedback'
import { observeUserMessageOverflow, TranscriptActiveContext } from './user-message-overflow'
import { TranscriptSearch } from './TranscriptSearch'
import { TranscriptFind, type TranscriptFindMemory } from './TranscriptFind'
import { createTranscriptRevealStore, TranscriptRevealContext, useRevealOpen } from './transcript-reveal'
import { useClockLabel } from '../../time/clock'
import { LiveRing } from './LiveRing'
import { transcriptInitialSourceRowCount } from './transcript-window'


export type LiveActivityState = { readonly _tag: 'Working' } | { readonly _tag: 'WaitingForAnswer' }
export interface TranscriptTurn {
  readonly id: string
  /** Omitted when the visible history starts mid-turn or has assistant-only output. */
  readonly prompt?: TextItem & { readonly role: 'user' }
  /** Exact runtime source references. The host owns turn/participant boundaries. */
  readonly items: readonly ConversationItem[]
  readonly work: WorkLogTurn
  /** Host-owned interaction state; the kit never infers a pending question from output text. */
  readonly activity?: LiveActivityState
  readonly senderCaptions?: Readonly<Record<string, string | undefined>>
  /**
   * The item that opened this episode when no person did: a wake or incoming mail. It is one of `items`
   * and draws first, above the work log, as the episode's trigger.
   */
  readonly triggerId?: string
  /** Host naming for each message item, by item id. */
  readonly participants?: Readonly<Record<string, MailParticipants>>
}
/** `Unavailable` is the whole status: one host reason line and no sync line. `detail` may be diagnostic, so it stays behind "Show details". `action` is the host's recovery, e.g. `{ label: 'Try again', onPress }`. */
export type TranscriptAvailability = { readonly _tag: 'Available' } | { readonly _tag: 'Unavailable'; readonly reason: string; readonly detail?: string; readonly action?: { readonly label: string; readonly onPress: () => void } }
/** The host's request for history before the loaded window: in flight, or failed with Retry. Diagnostics stay with the host. */
export type TranscriptOlderRequest = { readonly _tag: 'Loading' } | { readonly _tag: 'Failed' }
/** `HasOlder` is the boundary's presence; `Complete` removes it. `older` is optional request state for `onLoadEarlier`. */
export type TranscriptHistory = { readonly _tag: 'Complete' } | { readonly _tag: 'HasOlder'; readonly onLoadEarlier?: () => void; readonly older?: TranscriptOlderRequest }
export interface TranscriptProps {
  readonly turns: readonly TranscriptTurn[]
  readonly title: string
  /** Omit when the host already owns the conversation's header band. */
  readonly showHeader?: boolean
  /** With showHeader=false, preserve sync controls in the owning pane header's stable action slot. */
  readonly headerActionsHost?: HTMLElement | null
  readonly sync: SyncStatus
  /** Omit to subscribe only elapsed/sync labels to the host HumanClockProvider. */
  readonly now?: number
  readonly observedAt: number
  /** Optional host ownership of the final rendered turn; no additional DOM or rendering policy. */
  readonly tailRef?: React.Ref<HTMLElement>
  /** Open output in the host's detail surface. No split/controller is hidden in the kit. */
  readonly onOpenTool?: (call: WorkLogCall) => void
  /** Host-owned semantic detail for one recorded tool call, given the source item and its projected call. It replaces only the summary line; the raw input/output disclosure always remains. */
  readonly renderToolDetail?: (item: Extract<ConversationItem, { _tag: 'ToolCall' }>, call: WorkLogCall) => React.ReactNode | undefined
  readonly onRetrySync?: () => void
  readonly onRetryRun?: () => void
  readonly onRetrySend?: (itemId: string) => void
  /** Composer send availability; per-row Retry appears only while it is Ready (or not reported). */
  readonly sendAvailability?: ComposerAvailability
  /** Message Markdown images only; attachments use `attachmentImages`. */
  readonly resolveImage?: MarkdownImageResolver
  /** Opens a deferred image outside the page (for hosts whose CSP forbids inline remote images). */
  readonly onLoadImage?: MarkdownImageOpener
  /** The host's own inline references in message Markdown, such as a `doc/…` name it can open. */
  readonly renderInlineReference?: InlineReferenceRenderer
  /** Host-owned reads of image attachments on user and received (assistant or mailbox) prose, started only by Show image or Retry. Omit to keep every attachment a deferred card. */
  readonly attachmentImages?: TranscriptAttachmentImages
  readonly availability?: TranscriptAvailability
  readonly history?: TranscriptHistory
  readonly emptyState?: React.ReactNode | TranscriptEmptyState
  /** Host-owned first-publication body; an empty body can keep a switch lane stable. */
  readonly loadingState?: React.ReactNode
  /** Workbench conversation key for the owning surface's scroll memory. */
  readonly viewportKey?: string
  /** Host command, e.g. the newest own pending send: a changed key brings the latest turn into view. */
  readonly scrollToBottomKey?: string
  /** Contextual landmark names when multiple transcript views share one canvas. */
  readonly landmarkContext?: string
  /** Host-owned last child of a finished turn; never called while the turn is still running. */
  readonly renderTurnFooter?: (turn: TranscriptTurn) => React.ReactNode
  /** C8/E14: preserve the current seat prose header until the turn-language pick; hosts may compare the headerless path. */
  readonly showSeatHeader?: boolean
  /** False for a retained pane the host keeps mounted but hidden: viewport work and prompt measurement pause until it is shown again. */
  readonly active?: boolean
}
const SenderCaption = React.createContext<string | undefined>(undefined)
const MailNames = React.createContext<MailParticipants | undefined>(undefined)
const SeatHeader = React.createContext(true)
/** True for a prose block that continues the same sender's run inside one turn. */
const RunContinuation = React.createContext(false)
/** Whole-run text for the block that heads a multi-block prose run, so its single Copy covers every block. */
const RunCopyText = React.createContext<string | undefined>(undefined)
const RetrySend = React.createContext<TranscriptProps['onRetrySend']>(undefined)
/** False inside an episode: the episode's head carries its one timestamp, so rows carry none. */
const RowTimes = React.createContext(true)
const TurnFooterRenderer = React.createContext<TranscriptProps['renderTurnFooter']>(undefined)
const ToolDetailRenderer = React.createContext<TranscriptProps['renderToolDetail']>(undefined)
function TranscriptMessage() {
  const item = useAuiState(state => state.message.metadata.custom.item) as ConversationItem | undefined
  const summary = useAuiState(state => state.message.content.find(part => part.type === 'text')?.text ?? '')
  const caption = React.useContext(SenderCaption)
  const mailNames = React.useContext(MailNames)
  const continuation = React.useContext(RunContinuation)
  if (item === undefined || item._tag === 'ToolCall' || item._tag === 'Reasoning') return null
  if (item._tag === 'Text' && item.role !== 'system') return item.role === 'user' ? <UserMessage item={{ ...item, role: 'user' }} /> : <SeatProse item={{ ...item, role: 'assistant' }} senderLine={caption} continuation={continuation} />
  // Mail names its participants; a message with neither (an imported handoff) keeps the seat's prose.
  if (item._tag === 'Message') return item.text === undefined && item.from === undefined && item.to === undefined && item.author === undefined && mailNames === undefined
    ? <SeatProse item={item} senderLine={caption ?? item.sender?.label} />
    : <MailMessage item={item} participants={mailNames} />
  if (item._tag === 'Unsupported') return <MessagePrimitive.Root data-testid="transcript-message" data-item-id={item.id} data-item-kind={item._tag}><UnsupportedRecord item={item} /></MessagePrimitive.Root>
  if (item._tag === 'Event' && item.notice !== undefined) return <MessagePrimitive.Root data-testid="transcript-message" data-item-id={item.id} data-item-kind={item._tag}><HarnessNotice item={{ ...item, notice: item.notice }} weight="row" showTime={false} /></MessagePrimitive.Root>
  if (item._tag === 'UnknownEvent' && item.mixedContent === true) return <MixedContentMessage item={item} />
  if (item._tag === 'Text' || item._tag === 'Event') return <MessagePrimitive.Root data-testid="transcript-message" data-item-id={item.id} data-item-kind={item._tag} {...stylex.props(styles.semantic, styles.semanticProse)}><SemanticProse item={item} /></MessagePrimitive.Root>
  return <MessagePrimitive.Root data-testid="transcript-message" data-item-id={item.id} data-item-kind={item._tag} {...stylex.props(styles.semantic)}><span {...stylex.props(styles.semanticText)}>{summary}</span></MessagePrimitive.Root>
}
/** Mail delivered to the seat and harness messages are written as Markdown; an event's title is a label. */
function SemanticProse({ item }: { readonly item: Extract<ConversationItem, { _tag: 'Text' | 'Event' }> }) {
  return <>{item._tag === 'Event' && <span {...stylex.props(styles.semanticText)}>{item.title}</span>}<Markdown text={markdownBlock(item.text)} tone="muted" /></>
}
/** A message's text parts are Markdown by the transcript's contract. */
const MESSAGE_PARTS = { Text: ({ text }: { readonly text: string }) => <Markdown text={markdownBlock(text)} /> }
const messageComponents = { Message: TranscriptMessage }
/** Keep mixed bodies out of the collapsed DOM, using the same raw disclosure as tool inspection. */
function MixedContentMessage({ item }: { readonly item: Extract<ConversationItem, { _tag: 'UnknownEvent' }> }) {
  const [expanded, setExpanded] = React.useState(false)
  return <MessagePrimitive.Root data-testid="transcript-message" data-item-id={item.id} data-item-kind={item._tag} {...stylex.props(styles.mixed)}>
    <Disclosure isExpanded={expanded} onExpandedChange={setExpanded}>
      <div {...stylex.props(styles.mixedSummary)}><span>Message with internal records</span><span aria-hidden="true">·</span><Button slot="trigger" {...stylex.props(styles.mixedTrigger)}>{expanded ? 'Hide' : 'Show'}</Button></div>
      <DisclosurePanel>{expanded && <pre data-testid="mixed-content-body" {...stylex.props(styles.output)}><code>{JSON.stringify(item.data, null, 2)}</code></pre>}</DisclosurePanel>
    </Disclosure>
  </MessagePrimitive.Root>
}
export function UserMessage({ item }: { readonly item: TextItem & { readonly role: 'user' } }) {
  const onRetrySend = React.useContext(RetrySend)
  const [expanded, setExpanded] = React.useState(false)
  useRevealOpen(reveal => reveal.entryId === item.id, expanded, setExpanded)
  const active = React.useContext(TranscriptActiveContext)
  const [overflows, setOverflows] = React.useState(false)
  const text = React.useRef<HTMLDivElement>(null)
  const textId = React.useId()
  const long = item.text.length > 600 || item.text.split('\n').length > 8
  // The cut can land inside a construct; close it so the collapsed prompt keeps its formatting.
  const collapsed = React.useMemo(() => item.text.length > 600 ? completeStreamingTail(item.text.slice(0, 600)) : item.text, [item.text])
  React.useLayoutEffect(() => {
    const element = text.current
    if (element === null || expanded || !active) return
    return observeUserMessageOverflow(element, setOverflows)
  }, [item.text, expanded, active])
  return <MessagePrimitive.Root data-testid="user-message" data-item-id={item.id} data-send-state={(item.sendState?._tag ?? 'Sent').toLowerCase()} {...stylex.props(styles.user, item.sendState?._tag === 'Pending' && styles.userPending)}><div id={textId} ref={text} {...stylex.props(styles.userText, !expanded && styles.userClamp)}><Markdown text={markdownBlock(expanded ? item.text : collapsed)} tone={item.sendState?._tag === 'Pending' ? 'muted' : 'foreground'} hardBreaks /></div>{(long || overflows || expanded) && <Button aria-expanded={expanded} aria-controls={textId} onPress={() => setExpanded(!expanded)} {...stylex.props(styles.historyLoad, styles.userDisclosure)}>{expanded ? 'Show less' : 'Show full message'}</Button>}<TranscriptAttachments attachments={item.attachments} messageId={item.id} /><SendStatus state={item.sendState} onRetry={onRetrySend === undefined ? undefined : () => onRetrySend(item.id)} /></MessagePrimitive.Root>
}
/**
 * The callback must resolve only after the host has successfully copied the answer. Only a fulfilled copy sets
 * `data-copied`; a missing clipboard, a thrown or rejected write announces the failure instead.
 * `visible` reveals the action outside hover/focus; the held acknowledgement keeps it shown.
 */
export function CopyAnswerButton({ text, onCopy, visible = true }: { readonly text: string; readonly onCopy?: CopyAction; readonly visible?: boolean }) {
  const { state, copied, copy, tooltip, announcement } = useCopyFeedback(text, onCopy ?? (() => navigator.clipboard.writeText(text)))
  return <><Tooltip {...tooltip('Copy answer')}><Button aria-label={copied ? 'Copy answer: Copied' : 'Copy answer'} data-copied={copied ? 'true' : undefined} onPress={copy} {...stylex.props(styles.copy, (visible || copied) && styles.copyVisible)}><Icon name={copied ? 'check' : 'copy'} size={12} /></Button></Tooltip><VisuallyHidden><span role="status" aria-live="polite">{state.tag === 'failed' ? 'Could not copy answer. Check clipboard permissions and try again.' : announcement}</span></VisuallyHidden></>
}
/**
 * Copies this message's text, or the whole run's text for the block heading a multi-block prose run. Keyed by message id,
 * so a write that settles after unmount or a scope change (a new message id) never acknowledges the new scope.
 */
function RuntimeCopyAnswer({ visible }: { readonly visible: boolean }) {
  const runText = React.useContext(RunCopyText)
  const messageId = useAuiState(state => state.message.id)
  const ownText = useAuiState(state => state.message.content.filter(part => part.type === 'text').map(part => part.text).join('\n\n'))
  return <CopyAnswerButton key={messageId} text={runText ?? ownText} visible={visible} />
}
const messageTimeFormatter = new Intl.DateTimeFormat([], { hour: 'numeric', minute: '2-digit' })
/**
 * Sender, time and Copy share one header line; Copy shows on pointer hover or keyboard focus. A prose run shows the header once.
 * Received prose (an assistant answer or the paired body of incoming mail) shows its attachments below the text through the
 * same `TranscriptAttachments` as a user message, read for the attachment's carrier message, else this item's id.
 */
export function SeatProse({ item, senderLine, continuation = false }: { readonly item: (TextItem & { readonly role: 'assistant' }) | MessageItem; readonly senderLine?: string; readonly continuation?: boolean }) {
  const streaming = item._tag === 'Text' && item.streaming
  const completed = streaming ? NaN : Date.parse(item.at)
  const showSeatHeader = React.useContext(SeatHeader)
  const rowTimes = React.useContext(RowTimes)
  const header = !continuation && (item._tag !== 'Text' || showSeatHeader) && (senderLine !== undefined || !streaming)
  const [hovered, setHovered] = React.useState(false)
  return <MessagePrimitive.Root data-testid="agent-message" data-item-id={item.id} data-item-kind={item._tag} data-run-continuation={continuation || undefined} onMouseEnter={() => setHovered(true)} onMouseLeave={() => setHovered(false)} {...stylex.props(styles.answer)}>
    {header && <div data-testid="message-header" {...stylex.props(styles.answerHeader)}>
      {senderLine !== undefined && <p data-testid="message-sender" {...stylex.props(styles.sender)}>{senderLine}</p>}
      {!streaming && <span {...stylex.props(styles.answerMeta)}><RuntimeCopyAnswer visible={hovered} />{rowTimes && Number.isFinite(completed) && <time data-testid="message-time" dateTime={new Date(completed).toISOString()}>{messageTimeFormatter.format(completed)}</time>}</span>}
    </div>}
    {item._tag === 'Text' ? <><Markdown text={markdownBlock(item.text)} streaming={item.streaming} /><TranscriptAttachments attachments={item.attachments} messageId={item.id} /></> : <MessagePrimitive.Parts components={MESSAGE_PARTS} />}
  </MessagePrimitive.Root>
}
/** Human intent first; a failure shows its one-line reason, the diagnostic itself requires a second disclosure. A host's semantic detail stands in for the summary line; the raw disclosure always remains. */
export function ToolDetailPreview({ call, onOpen, semantic }: { readonly call: WorkLogCall; readonly onOpen?: (call: WorkLogCall) => void; readonly semantic?: React.ReactNode }) {
  const [rawOpen, setRawOpen] = React.useState(false)
  useRevealOpen(reveal => reveal.entryId === call.id && reveal.deep, rawOpen, setRawOpen)
  // A match past the preview's first lines has to be on screen to be highlighted in place.
  const [wholeOutput, setWholeOutput] = React.useState(false)
  useRevealOpen(reveal => reveal.entryId === call.id && reveal.deep, wholeOutput, setWholeOutput)
  const output = call.detail ?? (call.status === 'running' ? 'No output received yet.' : 'No output recorded.')
  const lines = output.split('\n')
  const remaining = Math.max(0, lines.length - Number(output.endsWith('\n')) - 4)
  const command = readableCommand(call)
  const showOutput = (call.detail ?? '') !== '' || call.rawInput === undefined
  return <div data-testid="tool-detail-preview" {...stylex.props(styles.preview)}>
    {semantic ?? <p>{call.summary === undefined ? call.title : <span data-search-tool-summary>{call.summary}</span>} · {call.status}</p>}
    {call.status === 'error' && call.detail !== undefined && <p data-testid="tool-error-reason" {...stylex.props(styles.errorReason)}>{errorReason(call.detail)}</p>}
    <Disclosure isExpanded={rawOpen} onExpandedChange={setRawOpen}>
      <Button slot="trigger" {...stylex.props(styles.previewOpen)}>Show raw input/output</Button>
      <DisclosurePanel>{rawOpen && <>
        {semantic != null && call.summary !== undefined && <p data-search-tool-summary>{call.summary}</p>}
        {call.rawInput !== undefined && <pre data-testid="tool-raw-input" data-search-tool-input {...stylex.props(styles.output)}><code {...stylex.props(styles.outputCode)}>{call.rawInput}</code></pre>}
        {command !== undefined ? <pre data-testid="tool-command" {...stylex.props(styles.output, styles.command)}><code>{command}</code></pre> : call.rawInput === undefined && call.argsSummary !== undefined && <pre {...stylex.props(styles.output)}>{call.argsSummary}</pre>}
        {showOutput && <pre data-search-tool-output={call.detail !== undefined && call.detail !== '' ? true : undefined} {...stylex.props(styles.output)}><code {...stylex.props(styles.outputCode)}><HighlightedSource code={onOpen === undefined || wholeOutput ? output : lines.slice(0, 4).join('\n')} language={workLogOutputLanguage(call)} /></code></pre>}
        {onOpen !== undefined && <div data-testid="tool-preview-actions" {...stylex.props(styles.previewActions)}>{remaining > 0 && !wholeOutput && <><span data-testid="tool-preview-remaining">+{remaining} {remaining === 1 ? 'line' : 'lines'}</span><span aria-hidden="true">·</span></>}<Button aria-label={`Open ${call.title} tool detail`} onPress={() => onOpen(call)} {...stylex.props(styles.previewOpen)}>Open</Button></div>}
      </>}</DisclosurePanel>
    </Disclosure>
  </div>
}
/** Source text an unadopted item can show without the runtime; absent when the item carries none. */
const strandedText = (item: ConversationItem): string | undefined => {
  switch (item._tag) {
    case 'Text': case 'Reasoning': return item.text
    case 'Message': return item.text === undefined ? item.title : `${item.title === undefined ? '' : `${item.title}\n`}${item.text}`
    case 'Notice': return item.detail === undefined ? item.text : `${item.text}\n${item.detail}`
    case 'Event': return `${item.title}\n${item.text}`
    case 'Status': return item.detail
    case 'ToolCall': case 'Usage': case 'UnknownEvent': case 'Unsupported': return undefined
    default: {
      const unhandled: never = item
      throw new Error(`Unrecognized stranded item: ${unhandled}`)
    }
  }
}
/** Fallback for an item the runtime never adopted: visible in place, never silently dropped. */
function StrandedItem({ item }: { readonly item: ConversationItem }) {
  if (item._tag === 'Unsupported') return <UnsupportedRecord item={item} />
  if (item._tag === 'Event' && item.notice !== undefined) return <HarnessNotice item={{ ...item, notice: item.notice }} weight="row" />
  if ((item._tag === 'Text' && item.role === 'system') || item._tag === 'Event') return <div data-testid="transcript-stranded" data-item-id={item.id} data-item-kind={item._tag} {...stylex.props(styles.semantic, styles.semanticProse)}><SemanticProse item={item} /></div>
  const text = strandedText(item)
  return <div data-testid="transcript-stranded" data-item-id={item.id} data-item-kind={item._tag} {...stylex.props(styles.semantic)}><span {...stylex.props(styles.semanticText)}>{text === undefined || text.trim() === '' ? 'Couldn\u2019t display this entry' : text}</span></div>
}
/** The content-bearing fields of an item; lifecycle and timing fields are left out. */
const itemContent = (item: ConversationItem): string => {
  switch (item._tag) {
    case 'Text': return `${item.text}\u0000${item.attachments.map(attachment => attachment.id).join('\u0000')}`
    case 'Reasoning': return item.text
    case 'Message': return `${item.title ?? ''}\u0000${item.text ?? ''}\u0000${item.attachments?.map(attachment => attachment.id).join('\u0000') ?? ''}`
    case 'Notice': return `${item.text}\u0000${item.detail ?? ''}`
    case 'Event': return `${item.title}\u0000${item.text}`
    case 'Status': return item.detail ?? ''
    case 'ToolCall': return `${JSON.stringify(item.input) ?? ''}\u0000${item.result === undefined ? '' : JSON.stringify(item.result.content) ?? ''}`
    case 'UnknownEvent': return JSON.stringify(item.data) ?? ''
    case 'Unsupported': return `${item.harness}\u0000${item.recordKind}\u0000${item.reason}\u0000${item.inspect.text}\u0000${item.inspect.bytes}\u0000${item.inspect.truncated}`
    case 'Usage': return ''
  }
}
const contentVersions = new WeakMap<ConversationItem, string>()
/** A cheap content revision (FNV-1a over the content fields), computed once per item object. */
const contentVersion = (item: ConversationItem): string => {
  const cached = contentVersions.get(item)
  if (cached !== undefined) return cached
  const content = itemContent(item)
  let hash = 0x811c9dc5
  for (let index = 0; index < content.length; index++) hash = Math.imul(hash ^ content.charCodeAt(index), 0x01000193)
  const version = `${item.id}:${content.length}:${(hash >>> 0).toString(36)}`
  contentVersions.set(item, version)
  return version
}
/** The work log lists tools separately, so all nonblank thoughts of one turn share a disclosure. */
const workLogReasoning = (items: readonly ConversationItem[]) => items.filter((item): item is Extract<ConversationItem, { _tag: 'Reasoning' }> => item._tag === 'Reasoning' && item.text.trim().length > 0)
/** Persistable anchor id of a turn's work summary; source history and the rendered summary must agree on it. */
const workSummaryId = (turnId: string) => JSON.stringify(['work-summary', turnId])
const workMemberIds = new WeakMap<TranscriptTurn, readonly string[]>()
/** Source tool calls and thoughts a turn's summary owns, computed once per source turn so memoized turns keep their props. */
const workMembers = (turn: TranscriptTurn): readonly string[] => {
  const cached = workMemberIds.get(turn)
  if (cached !== undefined) return cached
  const ids = turn.items.filter(item => item._tag === 'ToolCall' || item._tag === 'Reasoning').map(item => item.id)
  workMemberIds.set(turn, ids)
  return ids
}
/** When an episode began: its prompt or trigger, else its first entry. */
const turnStart = (turn: TranscriptTurn): number => {
  for (const item of [turn.prompt, ...turn.items]) {
    const at = item === undefined || item.at === undefined ? NaN : Date.parse(item.at)
    if (Number.isFinite(at)) return at
  }
  return NaN
}
const dayFormatter = new Intl.DateTimeFormat([], { weekday: 'long', month: 'long', day: 'numeric' })
const dayKey = (time: number) => Number.isFinite(time) ? new Date(time).toDateString() : undefined
/** A day separator before an episode that starts on another day than the one before it. */
function DaySeparator({ time }: { readonly time: number }) {
  return <div role="separator" data-testid="transcript-day" {...stylex.props(styles.day)}><time dateTime={new Date(time).toISOString().slice(0, 10)}>{dayFormatter.format(time)}</time></div>
}
const PreparedTurn = React.memo(function PreparedTurn({ turn, summaryMemberIds, now, stranded, onOpenTool, onRetryRun, landmarkContext, tailRef, newDay, startedAt }: { turn: TranscriptTurn; /** When the whole episode began; the mounted turn may have lost its trigger to the tail window. */ startedAt?: number; summaryMemberIds?: readonly string[]; now?: number; stranded?: ReadonlySet<string>; onOpenTool: TranscriptProps['onOpenTool']; onRetryRun?: () => void; landmarkContext?: string; tailRef?: React.Ref<HTMLElement>; newDay?: boolean }) {
  const renderFooter = React.useContext(TurnFooterRenderer)

  const renderToolDetail = React.useContext(ToolDetailRenderer)
  const detail = React.useCallback((call: WorkLogCall) => {
    const item = renderToolDetail === undefined ? undefined : turn.items.find((item): item is Extract<ConversationItem, { _tag: 'ToolCall' }> => item._tag === 'ToolCall' && item.id === call.id)
    return <ToolDetailPreview call={call} onOpen={onOpenTool} semantic={item === undefined ? undefined : renderToolDetail?.(item, call)} />
  }, [turn.items, renderToolDetail, onOpenTool])
  const reasoning = workLogReasoning(turn.items)
  const trigger = turn.triggerId === undefined ? undefined : turn.items.find(item => item.id === turn.triggerId)
  const shown = turn.items.filter(item => item._tag !== 'ToolCall' && item._tag !== 'Reasoning' && item !== trigger)
  const started = startedAt ?? turnStart(turn)
  const renderItem = (item: ConversationItem, continuation = false, runText?: string) => stranded?.has(item.id) ? <StrandedItem key={item.id} item={item} /> : <MailNames.Provider key={item.id} value={turn.participants?.[item.id]}><SenderCaption.Provider value={turn.senderCaptions?.[item.id]}><RunContinuation.Provider value={continuation}><RunCopyText.Provider value={runText}><ThreadPrimitive.Unstable_MessageById messageId={item.id} components={messageComponents} /></RunCopyText.Provider></RunContinuation.Provider></SenderCaption.Provider></MailNames.Provider>
  // A shown block continues a prose run when it and the block before it are the same sender's adopted prose.
  const continues = shown.map((item, index) => [item, shown[index - 1]].every(entry => entry?._tag === 'Text' && entry.role === 'assistant' && stranded?.has(entry.id) !== true) && turn.senderCaptions?.[item.id] === turn.senderCaptions?.[shown[index - 1]!.id])
  // The block heading a run carries the run's full text, blocks joined by a blank line, for its one Copy.
  const runTexts = shown.map((item, index) => {
    if (continues[index] || !continues[index + 1]) return undefined
    const end = continues.indexOf(false, index + 1)
    return shown.slice(index, end === -1 ? undefined : end).map(entry => entry._tag === 'Text' ? entry.text : '').join('\n\n')
  })
  // One timestamp per episode, at its head beside the prompt or trigger.
  const head = turn.prompt !== undefined
    ? (stranded?.has(turn.prompt.id) ? <StrandedItem item={turn.prompt} /> : <ThreadPrimitive.Unstable_MessageById messageId={turn.prompt.id} components={messageComponents} />)
    : trigger === undefined ? undefined : renderItem(trigger)
  return <section ref={tailRef} data-testid="transcript-turn" data-item-id={turn.id} data-source-ids={[...(turn.prompt === undefined ? [] : [turn.prompt.id]), ...turn.items.map(item => item.id)].join(' ')} data-scroll-anchor-container {...stylex.props(styles.turn)}><RowTimes.Provider value={false}>
    {newDay === true && Number.isFinite(started) && <DaySeparator time={started} />}
    <div data-testid="turn-head" {...stylex.props(styles.turnHead)}>
      <div {...stylex.props(styles.turnLead)}>{head}</div>
      {Number.isFinite(started) && <time data-testid="turn-time" dateTime={new Date(started).toISOString()} {...stylex.props(styles.turnTime)}>{messageTimeFormatter.format(started)}</time>}
    </div>
    {(turn.work.calls.length > 0 || reasoning.length > 0) && <WorkLogV1 turn={turn.work} summaryAnchorId={workSummaryId(turn.id)} summaryMemberIds={summaryMemberIds} ariaLabel={`Work log${Number.isFinite(started) ? `, started ${messageTimeFormatter.format(started)}` : ''}${landmarkContext ? `, ${landmarkContext}` : ''}`} listStyle={styles.workList} renderCallDetail={detail} previewCallDetail={!turn.work.running} interactiveCalls={onOpenTool !== undefined} hideLiveRow hideTime onRetry={onRetryRun} onOpenOutput={onOpenTool} expandedBody={reasoning.length > 0 ? <ThinkingRun key="thinking" items={reasoning} /> : undefined} />}
    {shown.map((item, index) => renderItem(item, continues[index]!, runTexts[index]))}
    {turn.work.running && <LiveActivity state={turn.activity ?? { _tag: 'Working' }} startedAt={turn.work.startedAt} now={now} />}
    {!turn.work.running && renderFooter?.(turn)}
  </RowTimes.Provider></section>
})
/** Host clock keeps elapsed labels deterministic and pauses with the conversation clock. */
export function LiveActivity({ state, startedAt, now }: { readonly state: LiveActivityState; readonly startedAt?: string; readonly now?: number }) {
  const duration = useClockLabel(now => {
    const started = Date.parse(startedAt ?? '')
    return Number.isFinite(started) && started <= now ? formatWorkDuration(now - started) || '<1s' : ''
  }, now)
  const waiting = state._tag === 'WaitingForAnswer'
  return <div data-testid="live-work" role="status" aria-label={waiting ? 'Waiting for your answer' : 'Response in progress'} {...stylex.props(styles.liveActivity)}><LiveRing spinning={!waiting} /><span data-testid="live-activity-label" {...stylex.props(styles.liveLabel)}>{`${waiting ? 'Waiting for your answer' : 'Working'}${duration === '' ? '' : ` · ${duration}`}`}</span></div>
}
// Hosts using AssistantRuntimeProvider directly expose no adoption store; its absence never changes.
const staticSubscription = () => () => undefined
/** One conversation item in source order; a turn's prompt is its own row. */
interface SourceRow { readonly item: ConversationItem; readonly turnId: string; readonly prompt: boolean }
/**
 * The mounted source rows for a viewport start: the suffix from `startIndex`, plus the earlier reasoning IDs of a
 * crossing work-log disclosure and the latest prompt when omitted. Interleaved tools and prose outside the suffix
 * stay deferred; completing a disclosure must not mount an unbounded work log.
 */
function sourceWindow(rows: readonly SourceRow[], startIndex: number): { readonly start: number; readonly shown: number; readonly pinnedPrompt?: string; readonly pinnedReasoning: readonly string[] } {
  const start = Math.min(startIndex, rows.length)
  let pinnedReasoning: readonly string[] = []
  if (start > 0 && start < rows.length) {
    const turnId = rows[start]!.turnId
    let turnStart = start
    let turnEnd = start + 1
    while (turnStart > 0 && rows[turnStart - 1]!.turnId === turnId) turnStart--
    while (turnEnd < rows.length && rows[turnEnd]!.turnId === turnId) turnEnd++
    const earlier = workLogReasoning(rows.slice(turnStart, start).map(row => row.item))
    const later = workLogReasoning(rows.slice(start, turnEnd).map(row => row.item))
    if (later.length > 0) pinnedReasoning = earlier.map(item => item.id)
  }
  let latestPrompt = rows.length - 1
  while (latestPrompt >= 0 && !rows[latestPrompt]!.prompt) latestPrompt--
  const pinnedPrompt = latestPrompt >= 0 && latestPrompt < start ? rows[latestPrompt]!.item.id : undefined
  return { start, shown: rows.length - start + pinnedReasoning.length + (pinnedPrompt === undefined ? 0 : 1), pinnedPrompt, pinnedReasoning }
}
/** Turns trimmed to the mounted source rows, with work-log calls trimmed alike so hidden tool rows never mount. Whole turns keep their identity, and every turn keeps its key, so revealing history only adds rows. */
function MountedTurns({ turns, sourceTurns, rowIds, start, pinnedPrompt, pinnedReasoning, stranded, now, tailRef, onOpenTool, onRetryRun, landmarkContext }: { readonly turns: readonly TranscriptTurn[]; readonly sourceTurns: ReadonlyMap<string, TranscriptTurn>; readonly rowIds: readonly string[]; readonly start: number; readonly pinnedPrompt?: string; readonly pinnedReasoning: readonly string[]; readonly stranded: ReadonlySet<string>; readonly now?: number; readonly tailRef?: React.Ref<HTMLElement>; readonly onOpenTool: TranscriptProps['onOpenTool']; readonly onRetryRun?: () => void; readonly landmarkContext?: string }) {
  const mounted = React.useMemo(() => {
    const ids = new Set(rowIds.slice(start))
    if (pinnedPrompt !== undefined) ids.add(pinnedPrompt)
    for (const id of pinnedReasoning) ids.add(id)
    return turns.flatMap(turn => {
      const prompt = turn.prompt !== undefined && ids.has(turn.prompt.id) ? turn.prompt : undefined
      const items = turn.items.filter(item => ids.has(item.id))
      if (prompt === turn.prompt && items.length === turn.items.length) return [turn]
      if (prompt === undefined && items.length === 0) return []
      const itemIds = new Set(items.map(item => item.id))
      return [{ ...turn, prompt, items, work: { ...turn.work, calls: turn.work.calls.filter(call => itemIds.has(call.id)) } }]
    })
  }, [turns, rowIds, start, pinnedPrompt, pinnedReasoning])
  const last = turns[turns.length - 1]
  // A day separator wherever an episode starts on another day than the episode before it.
  // Both come from the whole episode, so revealing history never moves its time or its day.
  const days = new Map<string, boolean>()
  const starts = new Map<string, number>()
  let previousDay: string | undefined
  for (const turn of turns) {
    const started = turnStart(turn)
    starts.set(turn.id, started)
    const day = dayKey(started)
    if (day === undefined) continue
    days.set(turn.id, previousDay !== undefined && day !== previousDay)
    previousDay = day
  }
  return mounted.map(turn => <PreparedTurn key={turn.id} newDay={days.get(turn.id)} startedAt={starts.get(turn.id)} turn={turn} summaryMemberIds={workMembers(sourceTurns.get(turn.id) ?? turn)} tailRef={turn.id === last?.id ? tailRef : undefined} now={turn.work.running ? now : 0} stranded={turn.prompt !== undefined && stranded.has(turn.prompt.id) || turn.items.some(item => stranded.has(item.id)) ? stranded : undefined} onOpenTool={onOpenTool} onRetryRun={onRetryRun} landmarkContext={landmarkContext} />)
}
/**
 * The window of rendered source rows takes no height: scrolling up prepends earlier rows, and this polite status tells
 * assistive tech what browser Find can reach. Show all prepares every loaded row for it, as a skip link that appears only
 * on keyboard focus. Once every row is rendered focus on the action moves to the conversation history.
 */
function HistoryRange({ total, start, shown, materializing, firstId, revealRow, scrollId }: { readonly total: number; readonly start: number; readonly shown: number; readonly materializing: boolean; readonly firstId: string; readonly revealRow: (id: string, preservePosition?: boolean) => void; readonly scrollId: string }) {
  const focusWithin = React.useRef(false)
  const statusId = React.useId()
  const complete = start === 0 && !materializing
  React.useLayoutEffect(() => {
    if (!complete || !focusWithin.current) return
    focusWithin.current = false
    document.getElementById(scrollId)?.focus({ preventScroll: true })
  }, [complete, scrollId])
  const hidden = total - shown
  const status = complete
    ? `All ${total} entries shown. Browser Find searches all loaded messages.`
    : materializing
      ? `Showing all ${total} entries… Browser Find may miss earlier entries until this finishes.`
      : `Latest ${shown} of ${total} entries shown. Browser Find misses ${hidden} earlier ${hidden === 1 ? 'entry' : 'entries'} until you choose Show all messages.`
  return <div role="group" aria-label="Conversation history range" aria-describedby={statusId} aria-busy={materializing} data-testid="transcript-history-controls" data-state={complete ? 'complete' : materializing ? 'materializing' : 'deferred'} onFocus={() => { focusWithin.current = true }} onBlur={event => { focusWithin.current = event.currentTarget.contains(event.relatedTarget) }} {...stylex.props(styles.historyBar)}>
    <span id={statusId} role="status" aria-live="polite" data-testid="transcript-history-range" {...stylex.props(styles.visuallyHidden)}>{status}</span>
    {!complete && <Button aria-label="Show all messages" data-testid="transcript-history-all" onPress={() => { if (!materializing) revealRow(firstId, true) }} {...stylex.props(styles.historyLoad, styles.historySkip)}>Show all</Button>}
  </div>
}
/** One in-lane row: one button changes label across idle, loading and failed, so focus stays on it. */
function HistoryBoundary({ history }: { readonly history: Extract<TranscriptHistory, { readonly _tag: 'HasOlder' }> }) {
  const loading = history.older?._tag === 'Loading', failed = history.older?._tag === 'Failed'
  const node = React.useRef<HTMLDivElement>(null)
  // Complete history (or a window that no longer starts at the top) removes this row; keep keyboard focus in the lane.
  React.useLayoutEffect(() => {
    const boundary = node.current
    return () => {
      if (boundary !== null && boundary.contains(boundary.ownerDocument.activeElement)) boundary.closest<HTMLElement>('[data-testid="transcript-scroll"]')?.focus({ preventScroll: true })
    }
  }, [])
  const load = loading || failed ? undefined : history.onLoadEarlier
  // A reader who has moved off the live tail and can see this row gets the next page. Opening a conversation sits at the tail,
  // so it fetches nothing; a failure waits for Retry, and the button stays for a lane with nothing to scroll.
  React.useEffect(() => {
    const boundary = node.current
    const lane = boundary?.closest<HTMLElement>('[data-testid="transcript-scroll"]')
    if (load === undefined || boundary == null || lane == null) return
    const scrolled = () => {
      const offTail = lane.scrollHeight - lane.clientHeight - lane.scrollTop > geometryNumbers.scrollEndTolerance
      if (offTail && boundary.getBoundingClientRect().bottom >= lane.getBoundingClientRect().top) load()
    }
    lane.addEventListener('scroll', scrolled, { passive: true })
    return () => lane.removeEventListener('scroll', scrolled)
  }, [load])
  return <div ref={node} data-testid="history-boundary" data-state={loading ? 'loading' : failed ? 'failed' : 'idle'} aria-busy={loading} {...stylex.props(styles.historyBoundary)}>
    <span aria-live="polite" data-testid="history-boundary-note" {...stylex.props(styles.historyNote)}>{failed ? "Couldn't load earlier messages" : 'Earlier messages not loaded'}</span>
    {/* Not RAC `isPending`: its focus announcement points a live-region `role="img"` at this button's id, which dangles once Complete removes the button. */}
    {history.onLoadEarlier !== undefined && <Button aria-disabled={loading || undefined} onPress={loading ? undefined : history.onLoadEarlier} {...stylex.props(styles.historyLoad, loading && styles.historyPending)}>{loading ? 'Loading earlier messages…' : failed ? 'Retry' : 'Load earlier messages'}</Button>}
  </div>
}
/** Locked U2·F3·Y3 presentation under the host's AssistantRuntimeProvider. */
export function Transcript({ turns, title, showHeader = true, headerActionsHost, sync, now, observedAt, onOpenTool, renderToolDetail, onRetrySync, onRetryRun, onRetrySend, sendAvailability, resolveImage, onLoadImage, renderInlineReference, attachmentImages, availability = { _tag: 'Available' }, history = { _tag: 'Complete' }, emptyState, loadingState, viewportKey, scrollToBottomKey, landmarkContext, renderTurnFooter, showSeatHeader = true, active = true, tailRef }: TranscriptProps) {
  const scrollId = React.useId()
  const [reveal] = React.useState(createTranscriptRevealStore)
  const [findMemory] = React.useState<TranscriptFindMemory>(() => new Map())
  const messages = useAuiState(state => state.thread.messages)
  // External-store runtimes adopt each snapshot in a passive effect, and the store publishes the
  // adopted messages a task later. A child effect runs before its provider's adoption effect, so
  // a host render alone cannot classify missing ids as stranded. The adoption store first confirms
  // the provider's current snapshot reached the runtime (hosts may filter entries out of it, so
  // this never depends on which ids it carries); ids the runtime holds remain pending until
  // publication, while ids it did not adopt may render their fallback. Only unpublished ids
  // subscribe to this seam.
  const [adoptionEpoch, setAdoptionEpoch] = React.useState<readonly TranscriptTurn[]>()
  const published = React.useMemo(() => new Set(messages.map(message => message.id)), [messages])
  const unpublished = React.useMemo(() => turns.flatMap(turn => [...(turn.prompt === undefined ? [] : [turn.prompt.id]), ...turn.items.map(item => item.id)]).filter(id => !published.has(id)), [turns, published])
  const adoption = React.useContext(RuntimeAdoptedIds)
  const getHeldKey = () => {
    const held = adoption?.get()
    return JSON.stringify({ held: held === undefined ? [] : unpublished.filter(id => held.has(id)), sourceAdopted: unpublished.length === 0 || adoption === undefined || adoption.isSnapshotAdopted() })
  }
  const heldKey = React.useSyncExternalStore(adoption?.subscribe ?? staticSubscription, getHeldKey, getHeldKey)
  const { runtimeIds, sourceAdopted } = React.useMemo(() => {
    const snapshot = JSON.parse(heldKey)
    return { runtimeIds: new Set<string>(snapshot.held), sourceAdopted: snapshot.sourceAdopted === true }
  }, [heldKey])
  React.useEffect(() => {
    if (sourceAdopted && adoptionEpoch !== turns && unpublished.some(id => !runtimeIds.has(id))) setAdoptionEpoch(turns)
  }, [turns, unpublished, runtimeIds, adoptionEpoch, sourceAdopted])
  const { committed, stranded } = React.useMemo(() => {
    const settled = new Set(adoptionEpoch?.flatMap(turn => [...(turn.prompt === undefined ? [] : [turn.prompt.id]), ...turn.items.map(item => item.id)]))
    const stranded = new Set<string>()
    const visible = (id: string) => {
      if (published.has(id)) return true
      if (!settled.has(id) || runtimeIds.has(id)) return false
      stranded.add(id)
      return true
    }
    const committed = turns.flatMap(turn => {
      // Adoption can split either end of a turn: a pending prompt or earlier item must
      // not hide the already-published suffix while an older page is being prepared.
      const prompt = turn.prompt !== undefined && visible(turn.prompt.id) ? turn.prompt : undefined
      const pendingItems = turn.items.some(item => !visible(item.id))
      if (prompt === turn.prompt && !pendingItems) return [turn]
      const items = pendingItems ? turn.items.filter(item => visible(item.id)) : turn.items
      if (prompt === undefined && items.length === 0) return []
      const itemIds = new Set(items.map(item => item.id))
      return [{ ...turn, prompt, items, work: { ...turn.work, calls: turn.work.calls.filter(call => itemIds.has(call.id)) } }]
    })
    return { committed, stranded }
  }, [published, turns, adoptionEpoch, runtimeIds])
  const strandedIds = [...stranded].join(', ')
  React.useEffect(() => {
    if (strandedIds !== '' && process.env.NODE_ENV !== 'production') console.warn(`Transcript: the runtime never adopted ${strandedIds}; showing a fallback row.`)
  }, [strandedIds])
  const imageOptions = React.useMemo(() => ({ resolveImage, onLoadImage, renderInlineReference }), [resolveImage, onLoadImage, renderInlineReference])
  // Source membership stays authoritative while adoption, folding or the tail window leaves rows unmounted. Only an
  // observed, complete, non-empty history can retire a saved anchor; waiting or partial history keeps it pending.
  // The id set is built only when a pending restore asks, never per streaming update.
  const anchorHistory = React.useMemo<ViewportAnchorHistory>(() => {
    if (history._tag !== 'Complete' || turns.length === 0) return { _tag: 'Partial' }
    let ids: ReadonlySet<string> | undefined
    return { _tag: 'Complete', includes: id => (ids ??= new Set(turns.flatMap(turn => [
      turn.id,
      ...(turn.prompt === undefined ? [] : [turn.prompt.id]),
      ...turn.items.map(item => item.id),
      ...(workMembers(turn).length > 0 ? [workSummaryId(turn.id)] : []),
    ]))).has(id) }
  }, [turns, history._tag])
  // A summary owns its turn's full source work, never only the adopted prefix it currently renders.
  const sourceTurns = React.useMemo(() => new Map(turns.map(turn => [turn.id, turn])), [turns])
  // New rows or changed content are news; send state, tool status and timing are metadata.
  const sourceRows = React.useMemo(() => committed.flatMap(turn => [...(turn.prompt === undefined ? [] : [{ item: turn.prompt, turnId: turn.id, prompt: true }]), ...turn.items.map(item => ({ item, turnId: turn.id, prompt: false }))]), [committed])
  // One viewport row per source row, so the initial bound and history reveal count conversation items rather than turns.
  const rows = React.useMemo(() => sourceRows.map(row => ({ id: row.item.id, version: contentVersion(row.item) })), [sourceRows])
  const rowIds = React.useMemo(() => rows.map(row => row.id), [rows])
  const progress = sync._tag === 'Progress' && sync.stage === 'reading' && sync.done !== undefined && sync.total !== undefined ? sync : undefined
  // Error copy is time-independent; time-bearing sync copy subscribes inside SyncLine.
  const failure = sync._tag === 'Failed' ? syncLine({ status: sync, label: 'conversation', now: now ?? observedAt, observedAt }) : undefined
  if (availability._tag === 'Unavailable') return <section aria-label={`Conversation unavailable${landmarkContext ? `, ${landmarkContext}` : ''}`} data-testid="transcript-unavailable" {...stylex.props(styles.frame, styles.empty)}>{showHeader && <header data-testid="transcript-header" {...stylex.props(styles.header)}><strong {...stylex.props(styles.title)}>{title}</strong></header>}<div {...stylex.props(styles.emptyBody)}>
    <TranscriptEmptyContent emptyState={{ title: availability.reason }} />
    {availability.action !== undefined && <Button data-testid="transcript-unavailable-action" onPress={availability.action.onPress} {...stylex.props(styles.unavailableAction)}>{availability.action.label}</Button>}
    {availability.detail !== undefined && <Disclosure {...stylex.props(styles.unavailableDetails)}><Button slot="trigger" {...stylex.props(styles.historyLoad)}>Show details</Button><DisclosurePanel><p data-testid="transcript-unavailable-detail" {...stylex.props(styles.unavailableDetail)}>{availability.detail}</p></DisclosurePanel></Disclosure>}
  </div></section>
  const empty = committed.length === 0 && turns.length === 0 && sync._tag === 'Live'
    ? <div aria-label="Empty conversation" data-testid="transcript-empty" {...stylex.props(styles.emptyBody)}><TranscriptEmptyContent emptyState={emptyState} /></div>
    : loadingState ?? <TranscriptSkeleton sync={sync} now={now} observedAt={observedAt} onRetrySync={onRetrySync} />
  const headerStatus = <>
    {progress !== undefined && <ProgressBar aria-label="Thread synchronization" value={progress.done} maxValue={progress.total} {...stylex.props(styles.progress)}><div {...stylex.props(styles.track)}><div {...stylex.props(styles.fill(`${progress.total === 0 ? 0 : progress.done! / progress.total! * 100}%`))} /></div></ProgressBar>}
    {committed.length > 0 && <SyncLine status={sync} label="conversation" now={now} observedAt={observedAt} onRetry={onRetrySync} inline={!showHeader} />}
  </>
  return <TranscriptRevealContext.Provider value={reveal}><TurnFooterRenderer.Provider value={renderTurnFooter}><ToolDetailRenderer.Provider value={renderToolDetail}><TranscriptActiveContext.Provider value={active}><SeatHeader.Provider value={showSeatHeader}><MarkdownImagePolicy.Provider value={imageOptions}><TranscriptAttachmentImagesContext.Provider value={attachmentImages}><RetrySend.Provider value={sendAvailability === undefined || sendAvailability._tag === 'Ready' ? onRetrySend : undefined}><ThreadPrimitive.Root aria-label={`Transcript${landmarkContext ? `, ${landmarkContext}` : ''}`} {...stylex.props(styles.frame)}>
    {showHeader && <header data-testid="transcript-header" {...stylex.props(styles.header)}><strong {...stylex.props(styles.title)}>{title}</strong>{headerStatus}</header>}
    {!showHeader && headerActionsHost != null && active ? createPortal(<div data-testid="transcript-header-status" {...stylex.props(styles.headerStatus, progress !== undefined && styles.headerProgress)}>{headerStatus}</div>, headerActionsHost) : null}
    <ErrorOverlayHost lane><EmbraceScrollViewport items={rows} anchorHistory={anchorHistory} stateKey={viewportKey} initialTailCount={transcriptInitialSourceRowCount} wholeMountSlack={0} scrollToBottomKey={scrollToBottomKey} active={active} id={scrollId} data-testid="transcript-scroll" role="group" aria-label="Conversation history" tabIndex={0} {...stylex.props(styles.lane)} contentProps={stylex.props(readingColumnStyles.column, styles.content)}
      renderHeader={(startIndex, revealRow, materializing) => {
        const bound = sourceWindow(sourceRows, startIndex)
        return <>
          {rows.length > transcriptInitialSourceRowCount && <HistoryRange total={rows.length} start={bound.start} shown={bound.shown} materializing={materializing} firstId={rowIds[0]!} revealRow={revealRow} scrollId={scrollId} />}
          <TranscriptFind key={viewportKey ?? ''} memoryKey={viewportKey ?? ''} memory={findMemory} scrollId={scrollId} turns={committed} rowIds={rowIds} revealRow={revealRow} history={history} active={active} reveal={reveal} />
        </>
      }}>
      {(startIndex, revealRow) => {
        const bound = sourceWindow(sourceRows, startIndex)
        return <>
          {history._tag === 'HasOlder' && bound.start === 0 && <HistoryBoundary history={history} />}
          {committed.length === 0 ? empty : <TranscriptSearch turns={committed} rowIds={rowIds} startIndex={bound.start} revealRow={revealRow} {...stylex.props(styles.timeline)}>
            <MountedTurns turns={committed} sourceTurns={sourceTurns} rowIds={rowIds} start={bound.start} pinnedPrompt={bound.pinnedPrompt} pinnedReasoning={bound.pinnedReasoning} stranded={stranded} now={now} tailRef={tailRef} onOpenTool={onOpenTool} onRetryRun={onRetryRun} landmarkContext={landmarkContext} />
          </TranscriptSearch>}
        </>
      }}
    </EmbraceScrollViewport>{failure?.tone === 'error' && <ErrorOverlay id={`sync-${failure.text}`} title={failure.text} detail="History stays on screen." onRetry={onRetrySync} />}</ErrorOverlayHost>
  </ThreadPrimitive.Root></RetrySend.Provider></TranscriptAttachmentImagesContext.Provider></MarkdownImagePolicy.Provider></SeatHeader.Provider></TranscriptActiveContext.Provider></ToolDetailRenderer.Provider></TurnFooterRenderer.Provider></TranscriptRevealContext.Provider>
}
const styles = stylex.create({
  frame: { minWidth: 0, minHeight: g.threadViewportMin, height: '100%', display: 'flex', flexDirection: 'column', overflow: 'hidden', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize },
  header: { height: g.band, minHeight: g.band, position: 'relative', display: 'flex', alignItems: 'center', gap: s.md, paddingInlineStart: `max(${s.lg}, env(safe-area-inset-left, 0px))`, paddingInlineEnd: `max(${s.lg}, env(safe-area-inset-right, 0px))`, flexShrink: 0, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  headerStatus: { position: 'relative', display: 'flex', alignItems: 'center', flexShrink: 1, gap: s.md, minWidth: 0, maxWidth: { default: 300, '@container thread-header (max-width: 440px)': 112 }, height: g.controlMd },
  headerProgress: { minWidth: g.controlLg },
  title: { flex: '1 1 0', minWidth: 0, fontWeight: t.weightMedium, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  lane: { flex: '1 1 0', minHeight: 0, minWidth: 0, overflowY: 'auto', overflowX: 'hidden', overflowAnchor: 'none' },
  // The shared reading column keeps rows and a reading-column composer on the same bounds.
  content: { paddingBlock: s.lg }, timeline: { display: 'flex', flexDirection: 'column', gap: s.xl, minWidth: 0 }, turn: { display: 'flex', flexDirection: 'column', gap: s.md, minWidth: 0 },
  workList: { maxHeight: 'none', overflowY: 'visible', overscrollBehavior: 'auto' },
  user: { display: 'flex', flexDirection: 'column', minWidth: 0, color: ink.fg, fontSize: t.bodySize, lineHeight: t.bodyLeading, borderLeftWidth: g.focusRing, borderLeftStyle: 'solid', borderLeftColor: accent.primary, paddingLeft: s.lg, paddingBlock: s.xs }, userText: { minWidth: 0, overflowWrap: 'anywhere' },
  // Eight prose lines, as the disclosure contract promises; padding and its optional button are outside this box.
  userClamp: { maxHeight: `calc(${t.bodyLeading} * 8)`, overflow: 'hidden' },
  userDisclosure: { paddingBlock: 0, lineHeight: t.metaLeading, alignSelf: 'flex-start' },
  // The outcome is what the reader came for: full contrast, never dimmer than the prompt.
  answer: { display: 'flex', flexDirection: 'column', gap: s.xs2, paddingBlock: s.xs2, color: ink.fg, flexShrink: 0 },
  turnHead: { display: 'flex', alignItems: 'flex-start', gap: s.md, minWidth: 0 },
  turnLead: { flex: '1 1 0', minWidth: 0 },
  turnTime: { flexShrink: 0, color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums' },
  day: { display: 'flex', alignItems: 'center', gap: s.md, color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, '::before': { content: '""', flex: '1 1 0', height: g.hairline, backgroundColor: border.border }, '::after': { content: '""', flex: '1 1 0', height: g.hairline, backgroundColor: border.border } },
  answerHeader: { display: 'flex', alignItems: 'center', gap: s.sm, height: t.metaLeading, minWidth: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  sender: { margin: 0, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium, color: ink.fgMuted },
  semantic: { display: 'flex', minWidth: 0, paddingBlock: s.xs2, color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  semanticText: { minWidth: 0, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  semanticProse: { flexDirection: 'column', gap: s.xs2, overflowWrap: 'anywhere' },
  mixed: { minWidth: 0, paddingBlock: s.xs2, color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  mixedSummary: { display: 'flex', alignItems: 'center', gap: s.xs },
  mixedTrigger: { minHeight: g.toolRow, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted, outlineOffset: g.focusOffset } },
  answerMeta: { display: 'flex', alignItems: 'center', gap: s.sm, flexShrink: 0, marginInlineStart: 'auto', fontVariantNumeric: 'tabular-nums' },
  // Copy keeps its place in the header and Tab order; pointer hover on the message or keyboard focus shows it.
  copy: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.icon, height: g.icon, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgMuted, cursor: 'pointer', opacity: { default: 0, ':focus': 1 }, transitionProperty: 'opacity', transitionDuration: { default: m.fast, '@media (prefers-reduced-motion: reduce)': '0s' }, transitionTimingFunction: m.ease, ':hover': { color: ink.fg, backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: `calc(-1 * ${g.focusRing})` } },
  copyVisible: { opacity: 1 },
  progress: { position: 'absolute', top: 0, left: 0, right: 0, height: g.progressSm }, track: { height: g.progressSm, backgroundColor: surface.rowActive, overflow: 'hidden' }, fill: (width: string) => ({ width, height: '100%', backgroundColor: accent.primary }),
  liveActivity: { display: 'flex', alignItems: 'center', gap: s.sm, minHeight: g.toolRow, color: ink.fgMuted, fontSize: t.uiSize },
  liveLabel: { color: ink.fgSoft },
  preview: { display: 'flex', flexDirection: 'column', alignItems: 'flex-start', marginInlineStart: g.textColumn, backgroundColor: surface.codeBg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.sm, paddingBlock: s.md, paddingInline: s.lg, gap: s.xs },
  output: { margin: 0, width: '100%', minWidth: 0, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontFamily: t.fontMono, fontSize: t.codeSize, lineHeight: t.metaLeading, color: ink.fgSoft }, outputCode: { fontFamily: t.fontMono, fontSize: t.codeSize },
  command: { boxSizing: 'border-box', marginBlockEnd: s.sm, padding: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, backgroundColor: surface.codeBg, color: ink.fg },
  errorReason: { margin: 0, maxWidth: '100%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fgSoft, fontSize: t.metaSize },
  previewActions: { display: 'flex', alignItems: 'center', gap: s.xs, minHeight: g.toolRow, color: ink.fgMuted, fontSize: t.metaSize }, previewOpen: { minHeight: g.toolRow, padding: 0, borderWidth: 0, backgroundColor: surface.transparent, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  empty: { backgroundColor: surface.washSubtle },
  emptyBody: { flex: '1 1 0', minHeight: 0, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: s.sm, padding: s.xl, textAlign: 'center' },
  historyBoundary: { display: 'flex', alignItems: 'center', justifyContent: 'center', gap: s.md, minHeight: g.controlMd, flexShrink: 0 },
  historyNote: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  historyBar: { position: 'relative', height: 0, flexShrink: 0, zIndex: 1 },
  // A skip link: off screen until keyboard focus lands on it.
  historySkip: { position: 'absolute', top: s.xs, left: s.lg, whiteSpace: 'nowrap', overflow: { default: 'hidden', ':focus': 'visible' }, width: { default: 1, ':focus': 'auto' }, height: { default: 1, ':focus': 'auto' }, clipPath: { default: 'inset(50%)', ':focus': 'none' }, backgroundColor: { default: surface.transparent, ':focus': surface.raised } },
  visuallyHidden: { position: 'absolute', width: 1, height: 1, padding: 0, margin: -1, overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap', borderWidth: 0 },
  historyLoad: { minHeight: g.controlSm, paddingInline: s.sm, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  historyPending: { cursor: 'progress', color: ink.fgMuted, ':hover': { color: ink.fgMuted } },
  unavailableAction: { minHeight: g.controlSm, marginTop: s.xs, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset } },
  unavailableDetails: { display: 'flex', flexDirection: 'column', alignItems: 'center', gap: s.xs, maxWidth: g.lane },
  unavailableDetail: { margin: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted, overflowWrap: 'anywhere' },
  userPending: { color: ink.fgMuted },
})
