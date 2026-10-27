import * as React from 'react'
import { markStart, measureEnd } from './perfMarks.ts'
import { useAtomValue } from '@effect/atom-react'
import { systemEventsPreference } from './conversationPreferences.ts'
import { EmbraceComposer } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceComposer.tsx'
import { EmbraceRuntimeProvider } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceRuntime.tsx'
import { Transcript } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Transcript.tsx'
import type { WorkLogCall } from '@fractal/ui/assistant-ui'
import { useConnection, useConversation, useConversationSync, useDataSource, useFeedInterest, useGrants, useNowSnapshot, usePausedNow } from '../data/react.tsx'
import { createConversationTranscript, frozenTranscript, openableImageUrl, transcriptObservedAt, transcriptSyncStatus, type ConversationTranscriptState } from './conversationTranscript.ts'
import { composerSendBinding, imageUploadAdapter, type SendRefusal } from './composerSend.ts'
import { nativeTurnFooter } from './NativeTurnFooter.tsx'
import { transcriptImagesFor } from './transcriptImages.ts'
import { spaceVars, geometryVars } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { LastSeen, ThreadUnavailable } from './ConversationStates.tsx'
import { LiveAgentAlerts } from '../conversation/alerts/AgentAlerts.tsx'
import { DOCUMENT_REF } from '../documents/document-port.ts'
import { documentReferences } from '../documents/DocumentLink.tsx'
import type { UxTelemetry } from '../telemetry/ux.ts'
import { inputPaintRegionMounted, registerInputPaintRegions, type InputPaintRegion } from '../telemetry/inputPaint.ts'
import type { ConversationPage, Feed } from '../data/source.ts'
import type * as Atom from 'effect/reactivity/Atom'
import type { ToolCallItem } from '../conversation/model.ts'
import { recordedToolDetail } from './semanticTool.tsx'
import { ThreadHeaderSlotContext } from '../shell/threadHeaderSlot.tsx'
import { AgentThreadTabs, type AgentThreadTabsHandle } from '../../../../packages/fractal-ui/src/assistant-ui/composition/AgentThreadTabs.tsx'
import { subagentThreadFocus } from './subagentThreadFocus.ts'

export const ConversationPane = (props: React.ComponentProps<typeof ConversationThreadPane>) => {
  const feed = useConversation(props.agentRef)
  const [selected, setSelected] = React.useState(props.agentRef)
  const children = feed._tag === 'Observed' ? feed.value.subagents ?? [] : []
  const threads = [
    { id: props.agentRef, label: 'Main' },
    ...children.flatMap((child, index) => child.conversation === undefined ? [] : [{ id: child.conversation.session_id, label: child.name ?? `Subagent ${index + 1}`, readOnly: true }]),
  ]
  const active = threads.find(thread => thread.id === selected) ?? threads[0]!
  const tabs = React.useRef<AgentThreadTabsHandle>(null)
  const focusRequest = useAtomValue(subagentThreadFocus(props.agentRef))
  React.useLayoutEffect(() => { if (focusRequest > 0) tabs.current?.focusSelected() }, [focusRequest])
  return <AgentThreadTabs ref={tabs} threads={threads} selectedId={active.id} onSelectionChange={setSelected}>
    <div hidden={active.id !== props.agentRef} style={{ display: active.id === props.agentRef ? 'flex' : 'none', flexDirection: 'column', flex: '1 1 0', minHeight: 0 }}>
      <ConversationThreadPane {...props} visible={(props.visible ?? true) && active.id === props.agentRef} />
    </div>
    {active.id === props.agentRef ? null : <ConversationThreadPane key={active.id} {...props} interest={undefined} agentRef={active.id} agentName={active.label} readOnly />}
  </AgentThreadTabs>
}

/** The follow owns content and the kit owns its presentation. The display-contents host
 * boundary observes asynchronous runtime adoption without inventing a kit commit hook.
 * Visibility lives only in this wrapper: toggling it never re-renders the memoized content,
 * so a retained pane is revealed without re-importing its runtime messages. */
const ConversationThreadPane = ({
  agentRef,
  agentName,
  onOpenTool,
  onOpenDocument,
  ux,
  visible = true,
  interest: shellInterest,
  readOnly = false,
}: {
  readonly agentRef: string
  /** Roster display name: the composition header and assistant sender captions. */
  readonly agentName: string
  /** Host-owned detail surface for a tool call opened from the transcript. */
  readonly onOpenTool: (call: WorkLogCall) => void
  /** Opens a `doc/…` named in the conversation; stable, so the memoized transcript keeps still. */
  readonly onOpenDocument?: (ref: string) => void
  readonly ux?: UxTelemetry
  /** A retained hidden pane keeps its DOM but releases visible follow demand (setVisible false). */
  readonly visible?: boolean
  /** Already acquired by the shell before this module loaded; standalone panes acquire their own. */
  readonly interest?: Atom.Atom<void>
  readonly readOnly?: boolean
}) => {
  const source = useDataSource()
  const showSystemEvents = useAtomValue(systemEventsPreference)
  // The shell owns early demand; a standalone pane owns the same source interest itself.
  const interest = React.useMemo(() => shellInterest ?? source.conversationInterest?.(agentRef), [shellInterest, source, agentRef])
  useFeedInterest({ interest, visible: shellInterest === undefined && visible })
  const feed = useConversation(agentRef)
  const connection = useConnection()
  // The same last-observed rule as the roster: a stale feed or a connection that is not live.
  const stale = feed._tag === 'Observed' && (feed.freshness === 'stale' || connection._tag !== 'Live')
  const projectTranscript = React.useMemo(createConversationTranscript, [])
  const state = projectTranscript(feed, { agentName, showSystemEvents })
  const observed = state._tag === 'Observed'
  const hasTurns = observed && state.turns.length > 0
  const boundary = React.useRef<HTMLDivElement>(null)
  React.useLayoutEffect(() => {
    const node = boundary.current
    if (node === null) return
    // Only the small composer is inert. Locking the entire transcript would restyle every
    // row in the switch frame. The boundary hides covered content from assistive technology.
    const composer = node.querySelector('textarea')?.closest('form')
    if (composer !== null && composer !== undefined) composer.inert = !visible
    const focused = node.ownerDocument.activeElement
    if (!visible && focused instanceof HTMLElement && node.contains(focused)) focused.blur()
  }, [visible, state._tag])
  React.useLayoutEffect(() => {
    const node = boundary.current
    if (node === null || !visible || !observed) return
    let cancelPaint: (() => void) | undefined
    let cancelOpen: (() => void) | undefined
    let cancelSwitch: (() => void) | undefined
    let tailFrame = 0
    const committed = () => {
      if (tailFrame !== 0) return
      const lane = node.querySelector('[data-testid="transcript-scroll"]')
      if (lane === null || lane.querySelector('[data-testid="transcript-placeholder"]') !== null) return
      if (lane.querySelector(hasTurns ? '[data-testid="transcript-turn"]' : '[data-testid="transcript-empty"]') === null) return
      if (cancelOpen === undefined) {
        cancelOpen = measureEnd('open_transcript', agentRef)
        cancelPaint = ux?.transcriptCommitted(agentRef)
      }
      // The viewport's own frame restores/pins its scroll position first. A restored
      // reading position is not a tail sample: wait until the actual tail is shown.
      tailFrame = requestAnimationFrame(() => {
        tailFrame = 0
        if (lane.scrollHeight - lane.clientHeight - lane.scrollTop > 2) return
        cancelSwitch = measureEnd('switch_to_tail', agentRef)
        observer.disconnect()
        node.removeEventListener('scroll', committed, true)
      })
    }
    // The external-store runtime adopts messages after the parent's layout commit.
    // Observe only this visible pane, and stop after its first real content commit.
    const observer = new MutationObserver(committed)
    observer.observe(node, { childList: true, subtree: true })
    node.addEventListener('scroll', committed, true)
    committed()
    return () => { observer.disconnect(); node.removeEventListener('scroll', committed, true); cancelAnimationFrame(tailFrame); cancelPaint?.(); cancelOpen?.(); cancelSwitch?.() }
  }, [agentRef, observed, hasTurns, ux, visible])
  // Diagnostics stay in data-wf-* attributes; the kit renders only the fixed unavailable copy.
  return <div ref={boundary} style={{ display: 'contents' }} aria-hidden={!visible}
    data-wf-unavailable={state._tag === 'Unavailable' ? state.classification : undefined}
    data-wf-unavailable-code={state._tag === 'Unavailable' ? state.code : undefined}
    onFocusCapture={event => { if (!visible) event.target.blur() }}
    onKeyDownCapture={event => { if (!visible) { event.preventDefault(); event.stopPropagation() } }}
    onClickCapture={event => { if (!visible) { event.preventDefault(); event.stopPropagation() } }}>
    <ConversationContent agentRef={agentRef} agentName={agentName} onOpenTool={onOpenTool} onOpenDocument={onOpenDocument} feed={feed} state={state} stale={stale} ux={ux} visible={visible} readOnly={readOnly} />
  </div>
}

const ConversationContent = React.memo(function ConversationContent({ agentRef, agentName, onOpenTool, onOpenDocument, feed, state, stale, ux, visible, readOnly }: {
  readonly stale: boolean
  readonly ux?: UxTelemetry
  readonly visible: boolean
  readonly readOnly: boolean
  readonly agentRef: string
  readonly agentName: string
  readonly onOpenTool: (call: WorkLogCall) => void
  readonly onOpenDocument?: (ref: string) => void
  readonly feed: Feed<ConversationPage>
  readonly state: ConversationTranscriptState
}) {
  const source = useDataSource()
  const grants = useGrants()
  const images = transcriptImagesFor(source)
  React.useSyncExternalStore(images.subscribe, images.snapshot, images.snapshot)
  React.useLayoutEffect(images.acquire, [images])
  const composer = React.useRef<HTMLDivElement>(null)
  const tail = React.useRef<HTMLElement | null>(null)
  const tailRef = React.useCallback((node: HTMLElement | null) => {
    const changed = node !== null && node !== tail.current
    tail.current = node
    if (changed && composer.current !== null) inputPaintRegionMounted(composer.current)
  }, [])
  React.useLayoutEffect(() => {
    const owner = composer.current
    if (owner === null) return
    return registerInputPaintRegions(owner, () => {
      const regions: InputPaintRegion[] = []
      const field = owner.querySelector('textarea,input,[contenteditable="true"]')
      if (field !== null) regions.push({ target: field })
      if (tail.current !== null) regions.push({ target: tail.current })
      return regions
    })
  }, [agentRef])
  const [refusal, setRefusal] = React.useState<SendRefusal>()
  const upload = source.attachments?.upload
  const attachmentAdapter = React.useMemo(() => upload === undefined ? undefined : imageUploadAdapter(upload), [upload])
  const binding = composerSendBinding({
    source, agentRef, grants,
    readable: state._tag === 'Observed',
    items: state._tag === 'Observed' ? state.items : [],
    refusal, onRefused: setRefusal, attachmentAdapter,
  })
  // The composer's Stop (and Esc) interrupts the agent's turn, as Interrupt in its actions does.
  // A refusal belongs to the run it answered (the running turn's id): it shows beside Stop while that run
  // lasts, clears when the run ends or another starts, and a late answer for an older run is dropped.
  const agentControls = readOnly ? undefined : source.agentControls
  const running = state._tag === 'Observed' && state.isRunning
  const runId = state._tag === 'Observed' && state.isRunning ? state.turns.findLast(turn => turn.work.running)?.id ?? '' : undefined
  const [stopRefusal, setStopRefusal] = React.useState<{ readonly runId: string; readonly detail: string }>()
  if (stopRefusal !== undefined && stopRefusal.runId !== runId) setStopRefusal(undefined)
  // The run on screen when an interrupt answer lands; a closure would hold the run of the press.
  const currentRun = React.useRef(runId)
  React.useLayoutEffect(() => { currentRun.current = runId }, [runId])
  const onCancel = React.useMemo(() => agentControls === undefined ? undefined : async () => {
    setStopRefusal(undefined)
    const result = await agentControls.interrupt(agentRef)
    // An answer for an older run must not touch the current run's refusal.
    if (result._tag === 'Refused' && runId !== undefined && currentRun.current === runId) setStopRefusal({ runId, detail: result.detail })
  }, [agentControls, agentRef, runId])
  const retryConversation = source.retryConversation
  // Only a failed (or not-yet-found) read can recover by re-acquiring the follow; refusals cannot.
  const retryUnavailable = state._tag === 'Unavailable' && retryConversation !== undefined && (state.classification === 'failed' || state.classification === 'not-found')
    && !(feed._tag === 'Unavailable' && feed.retryable === false) ? () => retryConversation(agentRef) : undefined
  const frozen = React.useMemo(() => stale && state._tag === 'Observed' ? frozenTranscript(state.turns) : undefined, [stale, state])
  const renderInlineReference = React.useMemo(() => onOpenDocument === undefined ? undefined : documentReferences(onOpenDocument), [onOpenDocument])
  // A Markdown link to a `doc/…` opens it in a tab instead of following a relative URL.
  const followDocumentLinks = (event: React.MouseEvent) => {
    const href = event.target instanceof Element ? event.target.closest('a[href]')?.getAttribute('href') ?? '' : ''
    if (onOpenDocument === undefined || !DOCUMENT_REF.test(href)) return
    event.preventDefault()
    event.stopPropagation()
    onOpenDocument(href)
  }
  // Recorded semantic evidence stands in for the kit summary line; the kit keeps raw input/output and Open beside it.
  const renderSemanticDetail = React.useCallback((item: ToolCallItem, call: WorkLogCall) => recordedToolDetail(item, call, () => onOpenTool(call)), [onOpenTool])
  return <EmbraceRuntimeProvider key={agentRef} options={{ ...binding.runtime, isRunning: running, ...(onCancel === undefined ? {} : { onCancel }) }}>
    {/* Bound the 100%-height kit frame to the space left above the composer. */}
    <div data-testid="conversation-history-host" onClickCapture={followDocumentLinks} style={{ flex: '1 1 0', minHeight: 0, minWidth: 0, overflow: 'hidden' }}>
    {state._tag === 'Unavailable' ? <ThreadUnavailable classification={state.classification} reason={state.availability._tag === 'Unavailable' ? state.availability.reason : ''} detail={state.availability._tag === 'Unavailable' ? state.availability.detail : undefined} onRetry={retryUnavailable} /> : <ConversationTranscript
      agentRef={agentRef}
      feed={feed}
      active={visible}
      viewportKey={agentRef}
      turns={frozen?.turns ?? (state._tag === 'Observed' ? state.turns : [])}
      scrollToBottomKey={feed._tag === 'Observed' ? feed.value.lastSendId : undefined}
      tailRef={tailRef}
      title={agentName}
      loadingState={<div data-testid="transcript-placeholder" role="status" aria-label="Loading conversation" />}
      onOpenTool={onOpenTool}
      renderToolDetail={renderSemanticDetail}
      renderTurnFooter={nativeTurnFooter}
      onRetrySync={retryConversation === undefined ? undefined : () => retryConversation(agentRef)}
      attachmentImages={{ state: images.state, load: (attachment, messageId) => { void images.load(attachment, messageId) } }}
      {...(renderInlineReference === undefined ? {} : { renderInlineReference })}
      onLoadImage={(src) => { const url = openableImageUrl(src); if (url !== undefined) window.open(url, '_blank', 'noopener,noreferrer') }}
      onRetrySend={!readOnly && grants.messageSend === 'granted' ? (id) => {
        if (feed._tag !== 'Observed') return
        const item = feed.value.items.find(item => item.id === id)
        if (item !== undefined) void binding.retry(item)
      } : undefined}
      {...(state._tag === 'Observed' && state.history._tag === 'HasOlder' ? { history: state.history } : {})}
      {...(state._tag === 'Observed' && state.emptyState !== undefined ? { emptyState: state.emptyState } : {})}
    />}
    </div>
    {frozen === undefined ? null : <LastSeen agentRef={agentRef} lastSeen={frozen.lastSeen} />}
    {!readOnly && <LiveAgentAlerts agentRef={agentRef} items={state._tag === 'Observed' ? state.items : []} ux={ux} />}
    {/* An unreadable conversation keeps its composer and draft; the binding names why sending waits. */}
    {!readOnly && <div ref={composer} data-testid="conversation-composer-dock" onInputCapture={(event) => {
      if (!(event.target instanceof HTMLTextAreaElement)) return
      markStart('keystroke_paint')
      measureEnd('keystroke_paint')
    }} style={{ flexShrink: 0, display: 'flex', flexDirection: 'column', justifyContent: 'flex-end', minHeight: `calc(${geometryVars.controlLg} + ${geometryVars.controlMd} + 3 * ${spaceVars.md} + 2 * ${geometryVars.hairline} + ${spaceVars.lg})`, paddingBottom: spaceVars.lg }}>
      <EmbraceComposer variant="C1" readingColumn conversationLabel={agentName} disabledReason={state._tag === 'Unavailable' && state.classification === 'not-found' ? 'Conversation not found' : binding.disabledReason} {...(binding.imageAttachments === undefined ? {} : { imageAttachments: binding.imageAttachments })} {...(stopRefusal === undefined ? {} : { stopRefusal: stopRefusal.detail })} />
    </div>}
  </EmbraceRuntimeProvider>
})

/**
 * Sync observations can change without rebuilding the runtime or its callbacks. While live, only
 * the transcript's time leaves tick; on transport loss they freeze at the last live sample.
 */
function ConversationTranscript({ agentRef, feed, ...props }: Omit<React.ComponentProps<typeof Transcript>, 'sync' | 'now' | 'observedAt'> & {
  readonly agentRef: string
  readonly feed: Feed<ConversationPage>
}) {
  const paused = usePausedNow()
  const snapshot = useNowSnapshot()
  const now = paused ?? snapshot
  const observation = useConversationSync(agentRef)
  const headerActionsHost = React.useContext(ThreadHeaderSlotContext)
  return <Transcript {...props} showHeader={headerActionsHost === null} headerActionsHost={headerActionsHost} sync={transcriptSyncStatus(observation, feed, now)} observedAt={transcriptObservedAt(observation, now)} {...(paused === undefined ? {} : { now: paused })} />
}

