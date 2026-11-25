import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import { useAtom, useAtomValue } from '@effect/atom-react'
import { Button, Input, Label, TextField } from 'react-aria-components'
import type { ThreadListQuery } from '@fractal/client'
import type { Attention, ThreadMessageEvent, ThreadStartContext, ThreadStartRecipient, ThreadSummary } from '@fractal/client/schema/read'
import type { ErrorEnvelope } from '@fractal/client/schema'
import { ago } from '../../../../clients/typescript/fractal-views/time.ts'
import { threadTitleFromAsk } from '../../../../clients/typescript/fractal-views/thread.ts'
import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useAgent, useDataSource, useGatewaySync, useNow } from '../data/react.tsx'
import { agentDisplayName, nameInSentence, personName } from '../data/agentName.ts'
import { ThreadPanel } from './ThreadPanel.tsx'
import { waiting, type Feed } from '../data/source.ts'
import { SystemNotice } from '../../../../packages/fractal-ui/src/assistant-ui/composition/SystemState.tsx'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { ThreadEditor } from './ThreadEditor.tsx'
import { Button as ActionButton } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Controls.tsx'
import { upgradeThreadEndpoint, type RetainedThreadStart, type ThreadObservation, type ThreadSendResult, type ThreadSource, type ThreadRefusal } from './threadSource.ts'
import { ThreadRefusalAlert } from './ThreadRefusalAlert.tsx'
import { useForegroundVisibility } from './useForegroundVisibility.ts'
import { awayVisit, enterUnreadVisit, holdUnreadVisit, planUnreadView, type UnreadVisit } from './unreadVisit.ts'
import { UnreadReminderControls } from './UnreadReminderControls.tsx'
import { ThreadRequestHistory } from './RelatedRequests.tsx'
import type { UnreadReminderRead } from './unread-action-port.ts'
import { threadAnchorAttribute, threadAnchorSelector, threadReadEligibleAttribute } from './threadAnchor.ts'

type OpenThread = { readonly onOpenThread?: ((id: string) => void) | undefined }

/** The same words the alert answer box uses for a device without control access. */
const readOnlyThread = 'Read-only on this device. Reply from a device with control access.'

/** The host places the same embedded thread group in its conversation reading lane. */
export function LiveAgentThreads({ agentId, onOpenThread }: OpenThread & { readonly agentId: string }) {
  return <ThreadGroup query={{ agent_id: agentId }} label="Threads" placement="pane" onOpenThread={onOpenThread} />
}

export function LiveMissionThreads({ missionId, onOpenThread }: OpenThread & { readonly missionId: string }) {
  return <ThreadGroup query={{ mission_id: missionId }} label="Threads" placement="pane" onOpenThread={onOpenThread} />
}

export function LiveRecentlyClosedThreads({ onOpenThread }: OpenThread) {
  return <ThreadGroup query={{ recently_closed: true, limit: 10 }} label="Recently closed threads" placement="sidebar" onOpenThread={onOpenThread} />
}

type Placement = 'pane' | 'sidebar'

function ThreadGroup({ query, label, placement, onOpenThread }: OpenThread & { readonly query: ThreadListQuery; readonly label: string; readonly placement: Placement }) {
  const source = useDataSource().threads
  return source === undefined ? null : <ThreadGroupContent source={source} query={query} label={label} placement={placement} onOpenThread={onOpenThread} />
}

const assumedOnline = Atom.make({ _tag: 'Online' } as const)

/**
 * Thread feeds share the gateway connection. Once it is lost they stay paused until the gateway reports Live
 * again: a successful HTTP read while the socket is still down does not bring new replies.
 */
function useUpdatesPaused(stale: boolean): boolean {
  const gateway = useGatewaySync()?.status._tag
  const network = useAtomValue(useDataSource().network ?? assumedOnline)
  const lost = network._tag === 'Offline' || gateway === 'Stale' || gateway === 'Failed'
  const [held, setHeld] = React.useState(false)
  const holding = lost || (held && gateway !== undefined && gateway !== 'Live')
  if (holding !== held) setHeld(holding)
  return stale || holding
}

/** Whether the gateway connection is live, or `undefined` where this source reports no gateway sync. */
function useGatewayLive(): boolean | undefined {
  const gateway = useGatewaySync()?.status._tag
  return gateway === undefined ? undefined : gateway === 'Live'
}

/**
 * An open thread's own reads (its detail and every message page it shows) can go stale while its section's list
 * stays live. Each mounted read registers here on its own while it is stale, and withdraws when it is live again or
 * unmounts, so the section's one "Updates paused" marker covers exactly what is on screen and offers Retry.
 */
const StaleThreadReads = React.createContext<((read: string) => () => void) | undefined>(undefined)

function useStaleThreadReads(): readonly [boolean, (read: string) => () => void] {
  const [stale, setStale] = React.useState<ReadonlySet<string>>(() => new Set())
  const report = React.useCallback((read: string) => {
    setStale(current => current.has(read) ? current : new Set(current).add(read))
    return () => setStale(current => {
      if (!current.has(read)) return current
      const next = new Set(current)
      next.delete(read)
      return next
    })
  }, [])
  return [stale.size > 0, report]
}

/** Registers this mounted read with its section while its observation is stale. */
function useReportStale(feed: Feed<unknown>) {
  const report = React.useContext(StaleThreadReads)
  const read = React.useId()
  const stale = feed._tag === 'Observed' && feed.freshness === 'stale'
  React.useEffect(() => stale && report !== undefined ? report(read) : undefined, [stale, report, read])
}

/**
 * "Threads 2 · Updates paused", named "Threads, 2, updates paused". The name is set on the heading or summary:
 * hidden separators between flex children are read with stray spaces ("Threads , 2"). The spaces between the parts
 * render nothing in the flex row, but keep the text's words apart ("Threads 2", not "Threads2"), so it matches the name.
 */
function CountedLabel({ label, count, paused }: { readonly label: string; readonly count: number | undefined; readonly paused: boolean }) {
  return <>
    <span {...stylex.props(styles.label)}>{label}</span>
    {count === undefined || count === 0 ? null : <>{' '}<span {...stylex.props(styles.count)}>{count}</span></>}
    {paused ? <>{' '}<span data-threads-paused="" {...stylex.props(styles.paused)}>· Updates paused</span></> : null}
  </>
}
const countedName = (label: string, count: number | undefined, paused: boolean) =>
  `${label}${count === undefined || count === 0 ? '' : `, ${count}`}${paused ? ', updates paused' : ''}`

/**
 * A one-line group opens independent embedded thread disclosures in the host's reading lane.
 * Recently closed sidebar rows navigate to a tab; embedded rows keep that navigation as an explicit action.
 */
function ThreadGroupContent({ source, query, label, placement, onOpenThread }: OpenThread & {
  readonly source: ThreadSource
  readonly query: ThreadListQuery
  readonly label: string
  readonly placement: Placement
}) {
  const narrow = React.useContext(NarrowLayoutContext)
  const { read, upgradeRequired, failure } = useAtomValue(source.access)
  const list = useAtomValue(source.list(query))
  const [readsStale, reportStale] = useStaleThreadReads()
  const paused = useUpdatesPaused(readsStale || (list._tag === 'Observed' && list.freshness === 'stale'))
  const [open, setOpen] = React.useState(false)
  // Only a complete first page states how many threads there are.
  const count = list._tag === 'Observed' && Option.isNone(list.value.next_cursor) ? list.value.items.length : undefined
  const content = upgradeRequired ? <ReadStatus feed={{ _tag: 'Unavailable', reason: 'unsupported', detail: upgradeThreadEndpoint }} subject="threads" />
    : accessFailed(failure) ? <ReadStatus feed={failure} subject="threads" onRetry={source.reload} />
    : !read || (list._tag === 'Observed' && list.value.items.length === 0 && Option.isNone(list.value.next_cursor)) ? null
    : <details aria-label={label} onToggle={event => setOpen(event.currentTarget.open)} {...stylex.props(styles.fold, placement === 'pane' && styles.strip)}>
      <summary aria-label={countedName(label, count, paused)} {...stylex.props(styles.foldSummary, placement === 'sidebar' && styles.foldSidebar, narrow && styles.foldTouch)}>
        <span {...stylex.props(styles.foldChevron)}><Icon name={open ? 'chevron-down' : 'chevron-right'} size={12} /></span>
        <CountedLabel label={label} count={count} paused={paused} />
      </summary>
      <div {...stylex.props(styles.foldBody, placement === 'sidebar' && styles.foldBodySidebar)}>
        {readsStale ? <ActionButton aria-label="Retry: Updates paused" onPress={source.reload}>Retry</ActionButton> : null}
        <ThreadListPage source={source} query={query} opensInTab={placement === 'sidebar'} onOpenThread={onOpenThread} />
      </div>
    </details>
  return <StaleThreadReads.Provider value={reportStale}>{content}</StaleThreadReads.Provider>
}

/** Collapsed rows share one bordered list so their columns line up; later pages join the same list. */
function ThreadListPage({ source, query, exclude, opensInTab = false, framed = true, onOpenThread }: OpenThread & {
  readonly source: ThreadSource
  readonly query: ThreadListQuery
  readonly exclude?: readonly string[] | undefined
  readonly opensInTab?: boolean
  readonly framed?: boolean
}) {
  const list = useAtomValue(source.list(query))
  const [showNext, setShowNext] = React.useState(false)
  if (list._tag !== 'Observed') {
    const status = <ReadStatus feed={list} subject="threads" onRetry={source.reload} />
    return framed ? status : <div {...stylex.props(styles.listFoot)}>{status}</div>
  }
  const next = Option.getOrUndefined(list.value.next_cursor)
  const rows = list.value.items.filter(thread => !exclude?.includes(thread.id))
  if (framed && rows.length === 0 && next === undefined) return null
  const page = <>
    {rows.map(thread => <LiveThread key={thread.id} source={source} thread={thread} inList opensInTab={opensInTab} onOpenThread={onOpenThread} />)}
    {next === undefined ? null : showNext
      ? <ThreadListPage source={source} query={{ ...query, cursor: next }} exclude={exclude} opensInTab={opensInTab} framed={false} onOpenThread={onOpenThread} />
      : <div {...stylex.props(styles.listFoot)}><ActionButton onPress={() => setShowNext(true)}>Load more threads</ActionButton></div>}
  </>
  return framed ? <div {...stylex.props(styles.list)}>{page}</div> : page
}

export function LiveInboxThreads({ alert, onOpenThread }: OpenThread & { readonly alert: Attention }) {
  return <LiveResolvedInboxThreads sourceId={alert.source_id} episode={alert.episode} title={alert.title} onOpenThread={onOpenThread} />
}

/**
 * Any exact source item — a current or resolved ask, an authored step, a typed or system notice — opens a new
 * discussion through the same authoritative start preview. Its recipient is only what that preview names.
 */
export function LiveResolvedInboxThreads({ sourceId, episode, title, onOpenThread }: OpenThread & {
  readonly sourceId: string
  readonly episode: string | undefined
  readonly title: string
}) {
  const source = useDataSource().threads
  if (source === undefined || episode === undefined) return null
  return <InboxThreadsContent key={JSON.stringify([sourceId, episode])} source={source}
    originSource={sourceId} episode={episode} title={title} onOpenThread={onOpenThread} />
}

/** The agent's display name, with the exact id only as the title: never a guessed role, never the id as text. */
function AgentName({ id }: { readonly id: string }) {
  const agent = useAgent(id)
  return <span title={id}>{nameInSentence(agentDisplayName({ id, name: agent._tag === 'Observed' ? agent.value?.name : undefined }))}</span>
}

/**
 * Who a first send on this item goes to, exactly as Fractal's start preview names it. Nothing else names a
 * recipient: a source without a preview reader offers no first send.
 */
type StartRecipientView =
  | { readonly _tag: 'Unassigned'; readonly explanation?: string | undefined }
  | { readonly _tag: 'Ready'; readonly recipient: ThreadStartRecipient }
  | { readonly _tag: 'Loading' }
  /** The generated recipient-retired error is terminal: no target and no default to offer. */
  | { readonly _tag: 'NoRecipient'; readonly explanation?: string | undefined }
  /** This source cannot read a start preview, so it has no recipient to name. */
  | { readonly _tag: 'NoPreview' }
  | { readonly _tag: 'Unavailable'; readonly feed: Extract<Feed<never>, { readonly _tag: 'Unavailable' }> }

// This literal must remain a registered generated error-envelope arm, not an open-ended wire string.
const unassignedCode: Extract<ErrorEnvelope, { readonly code: 'recipient-unassigned' }>['code'] = 'recipient-unassigned'
const startRecipientView = (feed: Feed<ThreadStartContext> | undefined): StartRecipientView => {
  if (feed === undefined) return { _tag: 'NoPreview' }
  if (feed._tag === 'Waiting') return { _tag: 'Loading' }
  if (feed._tag === 'Observed') return { _tag: 'Ready', recipient: feed.value.start_recipient }
  if (feed.code === unassignedCode) return { _tag: 'Unassigned', explanation: feed.explanation }
  if (feed.code === 'recipient-retired') return { _tag: 'NoRecipient', explanation: feed.explanation }
  return { _tag: 'Unavailable', feed }
}

const idleStartContext = Atom.make<Feed<ThreadStartContext>>(waiting)
const retiredNote = 'The recipient is retired, so no thread can start here.'
const noPreviewNote = 'This connection cannot preview a first-send recipient, so no thread can start here.'
const noRecipientSend = 'There is no recipient for this thread right now, so nothing was sent.'

function InboxThreadsContent({ source, originSource, episode, title, onOpenThread }: OpenThread & {
  readonly source: ThreadSource
  readonly originSource: string
  readonly episode: string
  readonly title: string
}) {
  const { read, actions, inboxOrigins, upgradeRequired, failure } = useAtomValue(source.access)
  const retained = useAtomValue(source.pendingStarts?.(originSource) ?? noPendingStarts)
    .filter(start => start.parameters.origin.episode === episode)
  const query = React.useMemo(() => ({ origin_source: originSource, origin_episode: episode }), [originSource, episode])
  const list = useAtomValue(source.list(query))
  // The exact source/episode preview is read only where a first send could be offered.
  const startContext = source.startContext
  const preview = useAtomValue(startContext !== undefined && read && actions.has('thread.start') ? startContext(originSource, episode) : idleStartContext)
  const recipient = startRecipientView(startContext === undefined ? undefined : preview)
  const narrow = React.useContext(NarrowLayoutContext)
  const [threadReadsStale, reportStale] = useStaleThreadReads()
  // A stale read keeps what it last showed; Retry reads every thread projection on this item again.
  const readsStale = (list._tag === 'Observed' && list.freshness === 'stale') || threadReadsStale
  const paused = useUpdatesPaused(readsStale)
  // One destination for this item's first send: a refusal and its draft stay put while the recipient is read again.
  const draftKey = `inbox:${JSON.stringify([originSource, episode])}`
  const draft = useAtomValue(source.draft(draftKey))
  const gatewayLive = useGatewayLive()
  const [starting, setStarting] = React.useState(false)
  // The recipient the person last saw when opening the draft or pressing Send; a different one is called out.
  const [reviewed, setReviewed] = React.useState<string>()
  // While the editor's own first send is in flight the editor stays mounted, so a refusal lands beside its draft;
  // only a send Fractal has not confirmed becomes a retained start with its own Retry.
  const [editorSending, setEditorSending] = React.useState(false)
  const [titleInput, setTitleInput] = React.useState<string>()
  const [focused, setFocused] = React.useState<string>()
  const headingId = React.useId()
  const startArea = React.useRef<HTMLDivElement>(null)
  const restoreStart = React.useRef(false)
  // Hiding the draft removes the focused editor; focus returns to the control that opened it.
  React.useLayoutEffect(() => {
    if (starting || !restoreStart.current) return
    restoreStart.current = false
    startArea.current?.querySelector('button')?.focus()
  }, [starting])
  const startInput = React.useCallback((input: HTMLTextAreaElement | null) => { input?.focus() }, [])
  if (upgradeRequired || (read && !inboxOrigins)) return <ReadStatus feed={{ _tag: 'Unavailable', reason: 'unsupported', detail: upgradeThreadEndpoint }} subject="threads" />
  if (accessFailed(failure)) return <ReadStatus feed={failure} subject="threads" onRetry={source.reload} />
  if (!read) return null
  // Only a complete first page states how many threads this item has.
  const count = list._tag === 'Observed' && Option.isNone(list.value.next_cursor) ? list.value.items.length : undefined
  // A start waits for the list: while it loads the sibling titles are unknown, and a refused or failed list is not offered one.
  const pending = editorSending ? retained.filter(start => start.draftKey !== draftKey) : retained
  const offerStart = pending.length === 0 && actions.has('thread.start') && list._tag === 'Observed'
  if (count === 0 && focused === undefined && pending.length === 0 && !offerStart) return null
  const askTitle = threadTitleFromAsk(title)
  // Two threads named after the same ask cannot be told apart, so a second one is named after its first line.
  const firstLine = draft.content.trim().split('\n', 1)[0] ?? ''
  const suggested = list._tag === 'Observed' && firstLine !== '' && list.value.items.some(item => item.title === askTitle)
    ? threadTitleFromAsk(firstLine) : askTitle
  const startTitle = threadTitleFromAsk(titleInput?.trim() || suggested)
  const saved = draft.content.trim() !== ''
  const ready = recipient._tag === 'Ready' ? recipient.recipient : undefined
  const changed = ready !== undefined && reviewed !== undefined && reviewed !== ready.agent_id
  const status = recipient._tag === 'Ready' ? null : <StartRecipientStatus recipient={recipient} onRetry={() => source.reloadStartContext?.(originSource, episode)} />
  // Sending names exactly the recipient on screen at that moment; a recipient Fractal no longer selects is refused
  // by the source, which reads this preview again without resending.
  const send = async (content: string): Promise<ThreadSendResult> => {
    if (ready === undefined) return { _tag: 'Refused', detail: noRecipientSend }
    setReviewed(ready.agent_id)
    setEditorSending(true)
    try {
      return await source.start(draftKey, { agent_id: ready.agent_id, origin: { source_id: originSource, episode }, title: startTitle, content })
    } finally {
      setEditorSending(false)
    }
  }
  return <StaleThreadReads.Provider value={reportStale}><section aria-labelledby={headingId} {...stylex.props(styles.inbox)}>
    <div {...stylex.props(styles.headingRow)}>
      <h3 id={headingId} aria-label={countedName('Threads', count, paused)} {...stylex.props(styles.inboxHeading)}><CountedLabel label="Threads" count={count} paused={paused} /></h3>
      {readsStale ? <ActionButton aria-label="Retry: Updates paused" onPress={source.reload}>Retry</ActionButton> : null}
      {/* The start control keeps one place at the end of the heading, however many threads open below it. */}
      {!offerStart || ready === undefined || starting ? null : <div ref={startArea} {...stylex.props(styles.startArea)}>
        <ActionButton variant="outline" onPress={() => { setReviewed(ready.agent_id); setStarting(true) }}>{saved ? 'Continue draft' : <><Icon name="plus" size={12} />Start thread</>}</ActionButton>
      </div>}
    </div>
    {focused === undefined ? null : <ThreadById source={source} threadId={focused} embedded focusComposer onOpenThread={onOpenThread} />}
    {pending.map(start => <PendingStartPanel key={start.draftKey} source={source} start={start} onSent={setFocused} />)}
    {/* A draft opens at the top of the list, where its thread lands once sent. */}
    {!offerStart ? null : starting
      ? <ThreadPanel thread={{ title: startTitle, state: 'open' }} initiallyExpanded onHideDraft={() => { restoreStart.current = true; setStarting(false) }}>
        <TextField value={titleInput ?? suggested} onChange={setTitleInput} {...stylex.props(styles.titleField)}>
          <Label>Thread title</Label>
          <Input {...stylex.props(styles.titleInput, narrow && styles.titleInputTouch)} />
        </TextField>
        {ready === undefined ? status : <>
          {changed ? <p role="status" data-start-recipient-changed="" {...stylex.props(styles.recipientChanged)}>
            The recipient changed to <AgentName id={ready.agent_id} />. Nothing has been sent; review the recipient and send again.
          </p> : null}
          <StartRecipient recipient={ready} lead="to" />
        </>}
        {/* The editor stays mounted while the recipient is read again, so its draft and refusal stay on screen. */}
        <ThreadEditor source={source} destination={draftKey} label={startTitle} inputRef={startInput} gatewayLive={gatewayLive} send={send}
          onSent={id => { setFocused(id); setStarting(false); setTitleInput(undefined); setReviewed(undefined) }} />
      </ThreadPanel>
      : ready === undefined ? status
      : count === 0 && focused === undefined ? <StartRecipient recipient={ready} lead="ask" /> : null}
    <ThreadListPage source={source} query={query} exclude={[...(focused === undefined ? [] : [focused]), ...retained.map(start => start.targetId)]} onOpenThread={onOpenThread} />
  </section></StaleThreadReads.Provider>
}

const basisLabel: Record<ThreadStartRecipient['basis']['_tag'], string> = {
  original: 'Original recipient',
  configured: 'Configured recipient',
  fallback: 'New recipient',
}

/**
 * The exact first-send recipient: its name, its exact agent id and the basis Fractal selected it on. A fallback
 * names the retired original agent separately: the retirement is Fractal's recorded `agent.retired` claim, never
 * an offline or stopped runtime.
 */
function StartRecipient({ recipient, lead }: { readonly recipient: ThreadStartRecipient; readonly lead: 'to' | 'ask' }) {
  const fallback = recipient.basis._tag === 'fallback' ? recipient.basis : undefined
  return <div data-start-recipient={recipient.basis._tag} {...stylex.props(styles.recipient)}>
    <p {...stylex.props(styles.recipientLine)}>
      <span>{lead === 'to' ? <>To <AgentName id={recipient.agent_id} /></> : <>Ask <AgentName id={recipient.agent_id} /> about this item without answering it.</>}</span>
      <code data-start-recipient-id="" {...stylex.props(styles.recipientId)}>{recipient.agent_id}</code>
      <span data-start-recipient-basis={recipient.basis._tag} {...stylex.props(styles.recipientBadge, fallback !== undefined && styles.recipientBadgeNew)}>{basisLabel[recipient.basis._tag]}</span>
    </p>
    {fallback === undefined ? null : <p data-start-recipient-original-row="" {...stylex.props(styles.recipientLine)}>
      <span>Originally for <AgentName id={fallback.original_agent_id} /></span>
      <code data-start-recipient-original="" {...stylex.props(styles.recipientId)}>{fallback.original_agent_id}</code>
      <span {...stylex.props(styles.recipientBadge)}>Retired</span>
    </p>}
  </div>
}

/** No recipient to show: loading, a refusal without a default, no preview reader, or a failed preview with Retry. */
function StartRecipientStatus({ recipient, onRetry }: { readonly recipient: Exclude<StartRecipientView, { readonly _tag: 'Ready' }>; readonly onRetry: () => void }) {
  switch (recipient._tag) {
    case 'Loading': return <SystemNotice kind="loading" scope="inline" title="Loading the recipient" />
    case 'Unassigned': return <SystemNotice kind="error" scope="inline" title="No recipient is configured. Set up a responsible agent before starting a thread." data-start-recipient="unassigned"
      {...(recipient.explanation === undefined ? {} : { explanation: recipient.explanation })} />
    case 'NoRecipient': return <SystemNotice kind="error" scope="inline" title={retiredNote} data-start-recipient="retired"
      {...(recipient.explanation === undefined ? {} : { explanation: recipient.explanation })} />
    case 'NoPreview': return <p data-start-recipient="none" {...stylex.props(styles.note)}>{noPreviewNote}</p>
    // Reading again cannot qualify a source Fractal does not know for this person.
    case 'Unavailable': return <ReadStatus feed={recipient.feed} subject="recipient" {...(recipient.feed.code === 'not-found' ? {} : { onRetry })} />
  }
}

/** Pending first sends survive the Inbox source that originally hosted their editor. */
export function LivePendingInboxStarts({ items, onOpenThread }: OpenThread & { readonly items: readonly Attention[] }) {
  const source = useDataSource().threads
  return source === undefined ? null : <PendingInboxStarts source={source} items={items} onOpenThread={onOpenThread} />
}

function PendingInboxStarts({ source, items, onOpenThread }: OpenThread & { readonly source: ThreadSource; readonly items: readonly Attention[] }) {
  const starts = useAtomValue(source.pendingStarts?.() ?? noPendingStarts).filter(start => !items.some(item =>
    item.source_id === start.parameters.origin.source_id && item.episode === start.parameters.origin.episode))
  const [focused, setFocused] = React.useState<string>()
  const [threadReadsStale, reportStale] = useStaleThreadReads()
  if (starts.length === 0 && focused === undefined) return null
  return <StaleThreadReads.Provider value={reportStale}><section aria-label="Thread recovery" {...stylex.props(styles.recovery)}>
    <h3 {...stylex.props(styles.note)}>Thread recovery</h3>
    {threadReadsStale ? <SystemNotice kind="stale" scope="inline" title="Updates paused" onRetry={source.reload} /> : null}
    {focused === undefined ? null : <ThreadById source={source} threadId={focused} embedded focusComposer onOpenThread={onOpenThread} />}
    {starts.map(start => <PendingStartPanel key={start.draftKey} source={source} start={start} onSent={setFocused} />)}
  </section></StaleThreadReads.Provider>
}

function PendingStartPanel({ source, start, onSent }: {
  readonly source: ThreadSource
  readonly start: RetainedThreadStart
  readonly onSent: (id: string) => void
}) {
  const retry = source.retryStart
  return <ThreadPanel thread={{ title: start.parameters.title, state: 'open' }} initiallyExpanded
    pendingChange="thread start" retryPending={start.state === 'sending'}
    {...(retry === undefined ? {} : { onRetryChange: async () => {
      const result = await retry(start.draftKey)
      if (result._tag === 'Refused') return result
      onSent(result.threadId)
      return undefined
    } })}>
    <p {...stylex.props(styles.note)}>To <AgentName id={start.parameters.agent_id} /></p>
    <div {...stylex.props(styles.message)}>{start.parameters.content}</div>
  </ThreadPanel>
}

/**
 * `true` focuses the composer once it mounts. `'when-lost'` does so only when nothing holds focus, as when the
 * control that opened this thread's tab unmounted with the view it lived in; switching tabs leaves focus alone.
 */
type FocusComposer = boolean | 'when-lost'

export function LiveThreadPane({ threadId, active = true }: { readonly threadId: string; readonly active?: boolean }) {
  const source = useDataSource().threads
  return source === undefined ? <p>This connection does not support threads.</p>
    : <div {...stylex.props(styles.standalone)}><ThreadById source={source} threadId={threadId} active={active} focusComposer="when-lost" /></div>
}

function ThreadById({ source, threadId, active = true, embedded = false, focusComposer = false, onOpenThread }: OpenThread & {
  readonly source: ThreadSource
  readonly threadId: string
  readonly active?: boolean
  readonly embedded?: boolean
  readonly focusComposer?: FocusComposer
}) {
  const { read, upgradeRequired, unknown, failure } = useAtomValue(source.access)
  const detail = useAtomValue(source.detail(threadId))
  const entryRead = useAtomValue(source.unreadActions?.reminder(threadId) ?? noReminder)
  const entryReminder = entryRead._tag === 'Known' ? entryRead.reminder : undefined
  // Capture at the route's entry cut, even while access/detail are still unknown.
  const visit = React.useRef<UnreadVisit>(active ? enterUnreadVisit(entryReminder) : awayVisit)
  const entry = React.useRef({ threadId, active })
  if (entry.current.threadId !== threadId || entry.current.active !== active) {
    entry.current = { threadId, active }
    visit.current = active ? enterUnreadVisit(entryReminder) : awayVisit
  }
  // The thread reports its own detail and each message page going stale; embedded, its section carries the marker.
  const [threadReadsStale, reportStale] = useStaleThreadReads()
  const paused = useUpdatesPaused(threadReadsStale)
  // A thread in its own tab keeps its status in the thread's measure, where its heading would be.
  const status = (notice: React.ReactNode) => embedded ? notice : <div {...stylex.props(styles.dedicatedNotice)}>{notice}</div>
  if (upgradeRequired) return status(<ReadStatus feed={{ _tag: 'Unavailable', reason: 'unsupported', detail: upgradeThreadEndpoint }} subject="thread" />)
  // Until Fractal answers which operations this connection holds, the thread is still loading, not unsupported.
  if (!read) return status(failure !== undefined && failure._tag !== 'Observed' ? <ReadStatus feed={failure} subject="thread" onRetry={source.reload} />
    : unknown ? <ReadStatus feed={{ _tag: 'Waiting' }} subject="thread" />
    : <p>This connection does not support threads.</p>)
  // Reading again cannot bring back a thread Fractal says does not exist.
  if (detail._tag === 'Unavailable' && detail.code === 'not-found') return status(<SystemNotice kind="empty" scope="inline" title="Fractal has no thread at this link." />)
  if (detail._tag !== 'Observed') return status(<ReadStatus feed={detail} subject="thread" onRetry={source.reload} />)
  const thread = <LiveThread key={threadId} source={source} thread={detail.value.summary} visitRef={visit} active={active} embedded={embedded} initiallyExpanded focusComposer={focusComposer} onOpenThread={onOpenThread} />
  if (embedded) return thread
  return <StaleThreadReads.Provider value={reportStale}>
    {/* A thread in its own tab carries the marker above its heading; Retry only when one of its reads went stale. */}
    {!paused ? null : <div data-threads-paused="" {...stylex.props(styles.dedicatedNotice)}>
      <SystemNotice kind="stale" scope="inline" connection={!threadReadsStale} title="Updates paused" {...(threadReadsStale ? { onRetry: source.reload } : {})} />
    </div>}
    {thread}
  </StaleThreadReads.Provider>
}

const noPendingChange = Atom.make<string | undefined>(undefined)
const idleDetail = Atom.make<Feed<ThreadObservation>>(waiting)
const noPendingStarts = Atom.make<readonly RetainedThreadStart[]>([])
const noReminder = Atom.make<UnreadReminderRead>({ _tag: 'Unavailable', detail: 'Unread reminders are not available on this connection.' })

function LiveThread({ source, thread, visitRef, active = true, embedded = true, inList = false, opensInTab = false, initiallyExpanded = false, focusComposer = false, onOpenThread }: OpenThread & {
  readonly source: ThreadSource
  readonly thread: ThreadSummary
  readonly visitRef?: React.RefObject<UnreadVisit>
  readonly active?: boolean
  readonly embedded?: boolean
  readonly inList?: boolean
  /** The row opens the dedicated tab instead of expanding in place, when a tab can be opened. */
  readonly opensInTab?: boolean
  readonly initiallyExpanded?: boolean
  readonly focusComposer?: FocusComposer
}) {
  const { actions } = useAtomValue(source.access)
  const [expanded, setExpanded] = React.useState(!embedded || initiallyExpanded)
  const detail = useAtomValue(expanded ? source.detail(thread.id) : idleDetail)
  const observation = detail._tag === 'Observed' ? detail.value : undefined
  // An open row shows its detail read's loading or failure instead of messages and controls it can no longer vouch for.
  const detailStatus = !expanded || detail._tag === 'Observed' ? null
    : detail._tag === 'Unavailable' && detail.code === 'not-found' ? <SystemNotice kind="empty" scope="inline" title="Fractal no longer has this thread." />
    : <ReadStatus feed={detail} subject="thread" onRetry={source.reload} />
  const summary = observation?.summary ?? thread
  useReportStale(detail)
  const agent = useAgent(summary.agent_id)
  const observedAgent = agent._tag === 'Observed' ? agent.value : undefined
  const agentName = observedAgent === undefined ? undefined : agentDisplayName({ id: summary.agent_id, name: observedAgent.name })
  const retryChange = source.retryChange
  const pendingChange = useAtomValue(source.pendingChange?.(thread.id) ?? noPendingChange)
  const markRead = summary.unread === true && actions.has('thread.read') ? source.markRead : undefined
  const viewport = useThreadViewport(expanded && (markRead !== undefined || source.unreadActions?.canMark === true))
  const foreground = useForegroundVisibility(active && expanded)
  const timeline = React.useCallback((node: HTMLDivElement | null) => {
    viewport.timeline(node)
    foreground.timeline(node)
  }, [viewport.timeline, foreground.timeline])
  const reminderRead = useAtomValue(expanded ? source.unreadActions?.reminder(thread.id) ?? noReminder : noReminder)
  const reminder = reminderRead._tag === 'Known' ? reminderRead.reminder : undefined
  const localVisit = React.useRef<UnreadVisit>(awayVisit)
  const visit = visitRef ?? localVisit
  const naturallyRead = React.useRef(new Set<string>())
  const [readFailure, setReadFailure] = React.useState<ThreadRefusal>()
  React.useLayoutEffect(() => {
    if (!active || !expanded) { visit.current = awayVisit; naturallyRead.current.clear(); return }
    // Unknown at entry captures nothing and remains sticky when a reminder arrives later.
    if (visit.current._tag === 'Away') visit.current = enterUnreadVisit(reminder)
    if (!foreground.foreground || observation === undefined) return
    if (source.unreadActions?.canMark && reminderRead._tag === 'Known') {
      const plan = planUnreadView(visit.current, { foreground: true, visibleMessageIds: foreground.visibleMessageIds, currentReminder: reminder })
      visit.current = plan.visit
      if (plan.observed_marks.length > 0) void source.unreadActions.clear({ threadId: thread.id, observed_marks: plan.observed_marks }).then(result => {
        if (result._tag !== 'Sent') setReadFailure({ detail: result.detail })
      }).catch(() => setReadFailure({ detail: 'The unread reminder change could not be confirmed. Refresh to check its current state.' }))
    }
    const seen = foreground.visibleMessageIds.filter(id => !naturallyRead.current.has(id))
    const anchor = seen.at(-1)
    if (markRead === undefined || anchor === undefined) return
    for (const id of seen) naturallyRead.current.add(id)
    void markRead(observation, anchor, seen).then(failure => { if (failure !== undefined) setReadFailure(failure) })
      .catch(() => setReadFailure({ detail: 'The read change could not be confirmed. Refresh to check its current state.' }))
  }, [active, expanded, foreground, observation, markRead, reminderRead, reminder, source, thread.id, visit])
  return <ThreadPanel thread={summary} pendingChange={pendingChange}
    unreadControls={source.unreadActions === undefined ? undefined : <UnreadReminderControls threadId={thread.id}
      port={source.unreadActions} visibleReplies={viewport.capture} timeline={viewport.container}
      onBeforeMark={() => { visit.current = holdUnreadVisit(visit.current) }} />}
    {...(retryChange === undefined ? {} : { onRetryChange: () => retryChange(thread.id) })}
    embedded={embedded} inList={inList} initiallyExpanded={initiallyExpanded} onExpansionChange={setExpanded}
    {...(opensInTab && onOpenThread !== undefined ? { onActivate: () => onOpenThread(thread.id) } : {})}
    {...(embedded || agentName === undefined ? {} : { origin: <>With <span title={summary.agent_id}>{nameInSentence(agentName)}</span></> })}
    {...(summary.message_count === undefined ? {} : { messageCount: summary.message_count })}
    unread={summary.unread === true}
    {...(onOpenThread === undefined || opensInTab ? {} : { onOpenInTab: () => onOpenThread(thread.id) })}
    {...(observation === undefined || !actions.has('thread.rename') ? {} : {
      onRename: (title: string, revision: string) => source.rename(observation, title, revision),
    })}
    {...(observation === undefined || !actions.has(summary.state === 'open' ? 'thread.close' : 'thread.reopen') ? {} : {
      onChangeState: (state: ThreadSummary['state']) => source.changeState(observation, state),
    })}
    {...(observation === undefined || markRead === undefined ? {} : {
      canMarkRead: viewport.canMark,
      onMarkRead: () => {
        const seen = viewport.capture(true)
        const anchor = seen.at(-1)
        return anchor === undefined
          ? Promise.resolve({ detail: 'Scroll to a visible reply before marking it read.' })
          : markRead(observation, anchor, seen)
      },
    })}
  >
    {detailStatus ?? <>
      {/* Explanatory history only: each related request keeps its own status; the conversation, unread and actions are unchanged. */}
      <ThreadRequestHistory source={source} threadId={thread.id} />
      <ThreadConversation source={source} thread={summary} agentName={agentName} canReply={actions.has('thread.reply')} observed={observation !== undefined}
        focusComposer={focusComposer} timelineRef={timeline} />
    </>}
    {readFailure === undefined ? null : <ThreadRefusalAlert refusal={readFailure} />}
  </ThreadPanel>
}

function useThreadViewport(enabled: boolean) {
  // The timeline mounts only once the thread's detail is observed, so observation follows the element itself.
  const [container, timeline] = React.useState<HTMLDivElement | null>(null)
  const [canMark, setCanMark] = React.useState(false)
  const capture = React.useRef<(readEligibleOnly: boolean) => string[]>(() => [])
  React.useLayoutEffect(() => {
    setCanMark(false)
    if (!enabled || container === null || typeof IntersectionObserver === 'undefined') return
    const tracked = new Set<Element>()
    const visible = new Set<Element>()
    let active = true
    const track = (entries: readonly IntersectionObserverEntry[]) => {
      if (!active) return
      for (const entry of entries) {
        if (tracked.has(entry.target) && entry.isIntersecting && entry.intersectionRatio >= 0.001) visible.add(entry.target)
        else visible.delete(entry.target)
      }
      setCanMark([...visible].some(row => row.hasAttribute(threadReadEligibleAttribute)))
    }
    // Viewport-rooted observation also clips against the pane’s ancestor scroll containers.
    const observer = new IntersectionObserver(track, { threshold: 0.001 })
    const rows = () => [...container.querySelectorAll(threadAnchorSelector)]
    const syncRows = () => {
      const current = new Set(rows())
      for (const row of tracked) {
        if (current.has(row)) continue
        observer.unobserve(row)
        visible.delete(row)
        tracked.delete(row)
      }
      for (const row of current) {
        if (tracked.has(row)) continue
        tracked.add(row)
        observer.observe(row)
      }
      setCanMark([...visible].some(row => row.hasAttribute(threadReadEligibleAttribute)))
    }
    const changes = new MutationObserver(syncRows)
    changes.observe(container, { childList: true, subtree: true, attributes: true, attributeFilter: [threadAnchorAttribute, threadReadEligibleAttribute] })
    syncRows()
    capture.current = readEligibleOnly => {
      track(observer.takeRecords())
      // The sticky control can overlap a row that still intersects the viewport.
      // Recheck current scroll clipping at the moment the person presses it.
      const bounds = threadViewportBounds(container)
      // Timeline DOM order is chronological, including earlier history pages.
      const seen = rows().flatMap(row => {
        if (!visible.has(row) || (readEligibleOnly && !row.hasAttribute(threadReadEligibleAttribute))) return []
        const id = row.getAttribute(threadAnchorAttribute)
        const rect = row.getBoundingClientRect()
        return id !== null && rect.top < bounds.bottom && rect.bottom > bounds.top
          && rect.left < bounds.right && rect.right > bounds.left ? [id] : []
      })
      return [...new Set(seen)].slice(-50)
    }
    return () => {
      active = false
      changes.disconnect()
      observer.disconnect()
      capture.current = () => []
    }
  }, [enabled, container])
  return { timeline, container, canMark, capture: (readEligibleOnly = false) => capture.current(readEligibleOnly) }
}

function threadViewportBounds(container: HTMLElement) {
  const bounds = { top: 0, bottom: window.innerHeight, left: 0, right: window.innerWidth }
  const clips = (overflow: string) => overflow !== '' && overflow !== 'visible'
  for (let element = container.parentElement; element !== null; element = element.parentElement) {
    const style = getComputedStyle(element)
    if (!clips(style.overflowX) && !clips(style.overflowY)) continue
    const rect = element.getBoundingClientRect()
    if (clips(style.overflowX)) {
      bounds.left = Math.max(bounds.left, rect.left + element.clientLeft)
      bounds.right = Math.min(bounds.right, rect.left + element.clientLeft + element.clientWidth)
    }
    if (clips(style.overflowY)) {
      bounds.top = Math.max(bounds.top, rect.top + element.clientTop)
      bounds.bottom = Math.min(bounds.bottom, rect.top + element.clientTop + element.clientHeight)
    }
  }
  const heading = container.closest('[data-thread-panel]')?.querySelector('[data-thread-read-heading]')
  if (heading) bounds.top = Math.max(bounds.top, heading.getBoundingClientRect().bottom)
  return bounds
}

function ThreadConversation({ source, thread, agentName, canReply, observed, focusComposer, timelineRef }: {
  readonly source: ThreadSource
  readonly thread: ThreadSummary
  readonly agentName: string | undefined
  /** The connection holds the reply grant; the composer still waits for the thread's own observation. */
  readonly canReply: boolean
  readonly observed: boolean
  readonly focusComposer: FocusComposer
  readonly timelineRef: React.Ref<HTMLDivElement>
}) {
  const [, setDraft] = useAtom(source.draft(thread.id))
  const gatewayLive = useGatewayLive()
  const input = React.useRef<HTMLTextAreaElement>(null)
  const didFocus = React.useRef(false)
  const attachInput = React.useCallback((element: HTMLTextAreaElement | null) => {
    input.current = element
    if (element === null || didFocus.current) return
    didFocus.current = true
    // Refs attach after the commit removed the old view, so a lost focus already reads as the body here.
    if (focusComposer === true || (focusComposer === 'when-lost' && (document.activeElement === null || document.activeElement === document.body))) element.focus()
  }, [focusComposer])
  const editable = canReply && observed
  const quote = editable ? (event: ThreadMessageEvent, sender: string) => {
    setDraft(draft => ({ ...draft, quote: { id: event.id, sender, excerpt: event.content } }))
    input.current?.focus()
  } : undefined
  return <>
    <div ref={timelineRef}><ThreadTimeline source={source} threadId={thread.id} agentId={thread.agent_id} agentName={agentName} onQuote={quote} /></div>
    {!canReply ? <p {...stylex.props(styles.note)}>{readOnlyThread}</p> : !editable ? null : <>
      {/* Replying never changes the lifecycle; only Reopen does. */}
      {thread.state === 'closed' ? <p {...stylex.props(styles.note)}>This thread is closed. Replying won’t reopen it.</p> : null}
      <ThreadEditor source={source} destination={thread.id} label={thread.title} inputRef={attachInput} gatewayLive={gatewayLive}
        send={(content, quoteId) => source.reply({ target_id: thread.id, content, ...(quoteId === undefined ? {} : { thread_quote_id: quoteId }) })} />
    </>}
  </>
}

type Quote = ((event: ThreadMessageEvent, sender: string) => void) | undefined

function ThreadTimeline({ source, threadId, cursor, direction, agentId, agentName, onQuote }: {
  readonly source: ThreadSource
  readonly threadId: string
  readonly cursor?: string
  readonly direction?: 'older' | 'newer'
  readonly agentId: string
  readonly agentName: string | undefined
  readonly onQuote: Quote
}) {
  const feed = useAtomValue(source.timeline(threadId, cursor))
  // Each page, the earlier ones included, keeps its last messages when it goes stale and says so to the section.
  useReportStale(feed)
  const { actor } = useAtomValue(source.access)
  const now = useNow()
  const [showOlder, setShowOlder] = React.useState(false)
  const [showNewer, setShowNewer] = React.useState(false)
  const rows = React.useRef<HTMLOListElement>(null)
  const revealedAnchor = React.useRef<string | undefined>(undefined)
  React.useLayoutEffect(() => {
    const anchor = feed._tag === 'Observed' && feed.value.anchor?.state === 'current' ? feed.value.anchor.message_id : undefined
    if (anchor === undefined || anchor === revealedAnchor.current) return
    const row = [...(rows.current?.querySelectorAll<HTMLElement>(threadAnchorSelector) ?? [])].find(row => row.getAttribute(threadAnchorAttribute) === anchor)
    if (row === undefined) return
    row.scrollIntoView?.({ block: 'center', behavior: 'auto' })
    revealedAnchor.current = anchor
  }, [feed])
  if (feed._tag !== 'Observed') return <ReadStatus feed={feed} subject="messages" onRetry={source.reload} />
  const next = direction === 'newer' ? undefined : Option.getOrUndefined(feed.value.next_cursor)
  const previous = direction === 'older' ? undefined : feed.value.previous_cursor
  return <>
    {next === undefined ? null : showOlder
      ? <ThreadTimeline source={source} threadId={threadId} cursor={next} direction="older" agentId={agentId} agentName={agentName} onQuote={onQuote} />
      : <ActionButton onPress={() => setShowOlder(true)}>Load earlier messages</ActionButton>}
    <ol ref={rows} {...stylex.props(styles.timeline)}>
      {feed.value.items.toReversed().map(event => {
        // Another person reads by their handle; only agents and Fractal's machinery go through the agent formatter.
        const sender = event.actor_id === actor || actor?.startsWith(`${event.actor_id}/session/`) ? 'You'
          : personName(event.actor_id) ?? agentDisplayName({ id: event.actor_id, name: event.actor_id === agentId ? agentName : undefined })
        const date = new Date(event.accepted_at_unix_ms)
        const iso = Number.isFinite(date.getTime()) ? date.toISOString() : undefined
        const time = iso === undefined ? null : <time dateTime={iso}>{ago(iso, now)} ago</time>
        // A lifecycle change is one quiet line; only messages carry the sender header.
        if (event.kind !== 'message') return <li key={event.id} data-thread-event={event.kind} {...stylex.props(stylex.defaultMarker(), styles.lifecycle)}>
          <span>{sender} {event.kind === 'thread.renamed' ? <>renamed “{event.previous_title}” to “{event.title}”</> : event.kind === 'thread.closed' ? 'closed the thread' : 'reopened the thread'}</span>
          {time === null ? null : <><span aria-hidden="true">·</span>{time}</>}
        </li>
        const quotedId = Option.getOrUndefined(event.thread_quote_id)
        const quoted = quotedId === undefined ? undefined : feed.value.items.find(
          (candidate): candidate is ThreadMessageEvent => candidate.kind === 'message' && candidate.id === quotedId,
        )
        return <li key={event.id} {...{ [threadAnchorAttribute]: event.id,
          [threadReadEligibleAttribute]: event.actor_id === agentId && feed.freshness === 'live' ? '' : undefined }} {...stylex.props(stylex.defaultMarker(), styles.event)}>
          <div {...stylex.props(styles.eventHeader)}>
            <strong title={sender === 'You' ? undefined : event.actor_id}>{sender}</strong>
            {time}
            {onQuote === undefined ? null : <Button aria-label={`Reply to ${nameInSentence(sender)}`} onPress={() => onQuote(event, nameInSentence(sender))} {...stylex.props(styles.reply)}>Reply</Button>}
          </div>
          {quotedId === undefined ? null
            : <blockquote {...stylex.props(styles.quote)}>{quoted?.content.slice(0, 160) ?? 'Reply to an earlier message'}</blockquote>}
          <div data-thread-message-content="" {...stylex.props(styles.message)}>{event.content}</div>
        </li>
      })}
    </ol>
    {previous === undefined ? null : showNewer
      ? <ThreadTimeline source={source} threadId={threadId} cursor={previous} direction="newer" agentId={agentId} agentName={agentName} onQuote={onQuote} />
      : <ActionButton onPress={() => setShowNewer(true)}>Load newer messages</ActionButton>}
  </>
}

/** A completed but broken access read. A refused credential is the connection's own notice, so lists do not repeat it. */
const accessFailed = (failure: Feed<never> | undefined): failure is Extract<Feed<never>, { readonly _tag: 'Unavailable' }> =>
  failure?._tag === 'Unavailable' && failure.reason === 'failed'

/** Loading is only loading; a refusal says what Fractal refused, and only a failed read offers Retry. */
function ReadStatus({ feed, subject, onRetry }: {
  readonly feed: Exclude<Feed<unknown>, { readonly _tag: 'Observed' }>
  readonly subject: 'threads' | 'thread' | 'messages' | 'recipient'
  readonly onRetry?: (() => void) | undefined
}) {
  if (feed._tag === 'Waiting') return <SystemNotice kind="loading" scope="inline" title={`Loading ${subject}`} />
  return <SystemNotice kind={feed.reason === 'unsupported' ? 'empty' : 'error'} scope="inline" title={feed.detail}
    {...(feed.explanation === undefined ? {} : { explanation: feed.explanation })}
    {...(feed.reason === 'failed' && onRetry !== undefined ? { onRetry } : {})} />
}

const styles = stylex.create({
  // The pinned strip and its fold follow AgentTodos: a hairline above, one summary line, a chevron slot and a muted count.
  fold: { minWidth: 0, fontSize: t.metaSize },
  strip: { flexShrink: 0, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: c.border },
  foldSummary: { display: 'flex', alignItems: 'center', gap: s.sm, minHeight: g.controlSm, boxSizing: 'border-box', paddingBlock: s.xs, marginInline: `calc(-1 * ${s.xs})`, paddingInline: s.xs, borderRadius: r.sm, color: c.fgSoft, cursor: 'pointer', listStyle: 'none', outline: 'none', ':hover': { backgroundColor: c.rowHover, color: c.fg }, ':focus-visible': { outline: `${g.focusRing} solid ${c.primary}` }, '::-webkit-details-marker': { display: 'none' } },
  // The sidebar summary starts where the alert rows' text does.
  foldSidebar: { marginInline: 0, paddingInline: s.row },
  foldTouch: { minHeight: g.touchTarget },
  foldChevron: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.iconSm, height: t.metaLeading, flexShrink: 0, color: c.fgMuted },
  // The open strip lists rows in the page's own scroll; it never adds a nested scroller.
  foldBody: { display: 'flex', flexDirection: 'column', gap: s.sm, minWidth: 0, paddingBlock: s.xs },
  foldBodySidebar: { paddingInline: s.row },
  label: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  count: { flexShrink: 0, fontWeight: t.weightRegular, color: c.fgMuted, fontVariantNumeric: 'tabular-nums' },
  paused: { flexShrink: 0, fontWeight: t.weightRegular, color: c.fgMuted, whiteSpace: 'nowrap' },
  list: { display: 'flex', flexDirection: 'column', minWidth: 0, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, borderRadius: r.sm },
  listFoot: { display: 'flex', alignItems: 'center', padding: s.xs, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: c.border },
  // The Inbox detail pane scrolls as one region; its threads never get a nested scroller.
  inbox: { display: 'flex', flexDirection: 'column', gap: s.sm, minWidth: 0, fontSize: t.metaSize },
  recovery: { display: 'flex', flexDirection: 'column', gap: s.sm, minWidth: 0, paddingInline: s.row, paddingBlock: s.sm, fontSize: t.metaSize },
  headingRow: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0 },
  inboxHeading: { display: 'flex', alignItems: 'baseline', gap: s.sm, flexGrow: 1, minWidth: 0, margin: 0, color: c.fgMuted, fontSize: t.metaSize, fontWeight: 600 },
  startArea: { display: 'flex', flexShrink: 0 },
  titleField: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0, color: c.fgMuted, fontSize: t.metaSize },
  titleInput: { flex: '1 1 0', minWidth: 0, height: g.controlSm, boxSizing: 'border-box', paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid',
    borderColor: c.borderStrong, borderRadius: r.sm, outline: { default: 'none', ':focus-visible': `${g.focusRing} solid ${c.primary}` }, backgroundColor: c.controlFill,
    color: c.fg, fontFamily: 'inherit', fontSize: t.metaSize },
  // Below 16px iOS Safari zooms the page when the field takes focus.
  titleInputTouch: { height: g.touchTarget, fontSize: t.touchInputSize },
  standalone: { position: 'absolute', inset: 0, overflowY: 'auto', backgroundColor: c.canvas, padding: s.md },
  dedicatedNotice: { boxSizing: 'border-box', width: '100%', maxInlineSize: g.lane, marginInline: 'auto' },
  timeline: { display: 'flex', flexDirection: 'column', gap: s.sm, padding: 0, margin: 0, listStyle: 'none' },
  event: { display: 'flex', flexDirection: 'column', gap: s.xs, color: c.fgMuted, fontSize: t.metaSize },
  lifecycle: { display: 'flex', alignItems: 'baseline', flexWrap: 'wrap', gap: s.xs, color: c.fgMuted, fontSize: t.metaSize },
  eventHeader: { display: 'flex', alignItems: 'baseline', gap: s.sm },
  reply: { opacity: { default: 0, ':focus-visible': 1, [stylex.when.ancestor(':hover')]: 1, '@media (hover: none)': 1 } },
  message: { color: c.fg, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  quote: { margin: 0, paddingInlineStart: s.sm, borderInlineStartWidth: 2, borderInlineStartStyle: 'solid', borderInlineStartColor: c.border,
    overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  note: { margin: 0, color: c.fgMuted, fontSize: t.metaSize },
  recipient: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0 },
  // The recipient's name, exact id and basis wrap under each other on a narrow pane; the badge stays whole.
  recipientLine: { display: 'flex', alignItems: 'baseline', flexWrap: 'wrap', columnGap: s.sm, rowGap: s.xs, margin: 0, minWidth: 0, color: c.fgMuted, fontSize: t.metaSize, overflowWrap: 'anywhere' },
  // The exact id is the recipient's identity: selectable, never truncated, wrapping anywhere on a phone.
  recipientId: { minWidth: 0, color: c.fgSoft, fontFamily: t.fontMono, fontSize: t.denseSize, overflowWrap: 'anywhere', userSelect: 'text' },
  recipientBadge: { flexShrink: 0, paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, borderRadius: r.sm,
    color: c.fgMuted, fontSize: t.metaSize, whiteSpace: 'nowrap' },
  // A fallback is the one basis to notice before sending: a stronger rule and weight, never a colour alone.
  recipientBadgeNew: { borderColor: c.borderStrong, color: c.fgSoft, fontWeight: 600 },
  // A changed recipient is the one thing to review before sending again: the attention rule marks it without colouring the words.
  recipientChanged: { margin: 0, paddingInlineStart: s.sm, borderInlineStartWidth: 2, borderInlineStartStyle: 'solid', borderInlineStartColor: c.attention,
    color: c.fg, fontSize: t.metaSize, overflowWrap: 'anywhere' },
})
