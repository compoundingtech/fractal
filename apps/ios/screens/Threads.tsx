import { useCallback, useEffect, useMemo, useRef, useState, type Ref } from 'react';
import * as Crypto from 'expo-crypto';
import { Alert, Pressable, ScrollView, TextInput, useWindowDimensions, View } from 'react-native';
import { useFocusEffect } from '@react-navigation/native';
import { ago, INLINE_MAX_BYTES, threadTitleFromAsk, utf8Bytes } from '@fractal/views';
import { ClientError, type CollectionWindow, type ThreadCollectionContext, type ThreadSummary, type ThreadTimelineEvent } from '../../../clients/typescript/fractal-client';
import { THREAD_WINDOW_SIZE, type ThreadWindowHandlers } from '../feed';
import { rememberBounded } from '../boundedCache';
import { agentName } from '../agentsView';
import type { RootScreen } from '../navigation';
import { errorText, threadDraftKey, threadStartScope, useStore, type ThreadEditResult, type ThreadStartOrigin } from '../store';
import { theme } from '../theme';
import { Button, Field, Markdown, SchemeScreen, T, useTextAppearance } from '../ui';

type ThreadPage<T> = { items: T[]; next_cursor: string | null };
type ReadPage<T> = (cursor: string | undefined, signal: AbortSignal) => Promise<{ value: ThreadPage<T> }>;
type FollowPage<T extends { id: string }> = (handlers: ThreadWindowHandlers<T>) => { close(): void } | undefined;

function sameHead<T extends { id: string }>(left: readonly T[], right: readonly T[]): boolean {
  return left.length === right.length && left.every((item, index) => item.id === right[index]?.id);
}

// The live window owns the head; HTTP owns continuation cursors. Never show an
// older tail unless its first page still aligns with the live head.
function useThreadPage<T extends { id: string }>(read: ReadPage<T>, refresh: number, follow?: FollowPage<T>, onHeadChange?: () => void) {
  const active = useRef<AbortController | null>(null);
  const [result, setResult] = useState<{ read: ReadPage<T>; head: T[]; page: ThreadPage<T>; issue?: string }>();
  const [live, setLive] = useState<{ read: ReadPage<T>; follow: FollowPage<T>; window?: CollectionWindow<T>; issue?: string; refused?: boolean }>();
  const [loading, setLoading] = useState(true);
  const visible = result?.read === read ? result : undefined;
  const visibleLive = live?.read === read && live.follow === follow ? live : undefined;
  const load = useCallback(async (prior?: { head: T[]; page: ThreadPage<T> }) => {
    active.current?.abort();
    const request = new AbortController();
    active.current = request;
    setLoading(true);
    try {
      const { value } = await read(prior?.page.next_cursor ?? undefined, request.signal);
      if (!request.signal.aborted) {
        const page = prior ? { ...value, items: [...prior.page.items, ...value.items] } : value;
        setResult({ read, head: prior?.head ?? value.items, page });
      }
    } catch (error) {
      if (!request.signal.aborted) {
        const canKeepPrior = !(error instanceof ClientError && [401, 403, 404, 409].includes(error.status));
        setResult({ read, head: canKeepPrior && prior ? prior.head : [], page: canKeepPrior && prior ? prior.page : { items: [], next_cursor: null }, issue: errorText(error) });
        if (error instanceof ClientError && [401, 403, 404].includes(error.status)) {
          setLive(previous => previous?.read === read ? { ...previous, window: undefined, refused: true, issue: errorText(error) } : previous);
        }
      }
    } finally {
      if (!request.signal.aborted) setLoading(false);
    }
  }, [read]);
  useFocusEffect(useCallback(() => {
    void load();
    return () => active.current?.abort();
  }, [load, refresh]));
  useFocusEffect(useCallback(() => {
    if (!follow) return;
    let current: CollectionWindow<T> | undefined;
    const held = follow({
      onWindow: window => {
        const changed = !current || !sameHead(current.items, window.items) || current.hasMore !== window.hasMore;
        if (changed) onHeadChange?.();
        current = window;
        setLive({ read, follow, window });
        if (changed) void load();
      },
      onIssue: (issue, refused) => {
        if (refused) {
          active.current?.abort();
          setResult(undefined);
          setLoading(false);
        }
        setLive({ read, follow, issue, refused, window: refused ? undefined : current });
      },
    });
    return () => held?.close();
  }, [follow, load, onHeadChange, read]));

  const liveWindow = visibleLive?.window;
  const aligned = !liveWindow || !!visible && sameHead(visible.head, liveWindow.items);
  const tail = aligned && visible && liveWindow ? visible.page.items.slice(visible.head.length) : [];
  const items = visibleLive?.refused ? [] : liveWindow ? [...liveWindow.items, ...tail] : visible?.page.items ?? [];
  return {
    items,
    issue: visibleLive?.issue || visible?.issue,
    loading,
    reload: () => void load(),
    more: aligned && !visibleLive?.refused && visible?.page.next_cursor ? () => void load(visible) : undefined,
  };
}

type ThreadClient = NonNullable<ReturnType<typeof useStore>['threadClient']>;


export function AgentThreads({ agentId, openThread }: { agentId: string; openThread: (id: string) => void }) {
  return <ThreadGroup context={{ agent_id: agentId }} openThread={openThread} />;
}

export function ThreadGroup({ context, openThread, title = 'Threads', excludeId }: {
  context: ThreadCollectionContext; openThread: (id: string) => void; title?: string; excludeId?: string;
}) {
  const { threadClient } = useStore();
  return threadClient ? <ContextThreadList client={threadClient} context={context} openThread={openThread} title={title} excludeId={excludeId} /> : null;
}

function ContextThreadList({ context, client, openThread, title, excludeId }: {
  context: ThreadCollectionContext; client: ThreadClient; openThread: (id: string) => void; title: string; excludeId?: string;
}) {
  const { threadRefresh, feed } = useStore();
  const agent = 'agent_id' in context ? context.agent_id : undefined;
  const mission = 'mission_id' in context ? context.mission_id : undefined;
  const ask = 'origin_ask' in context ? context.origin_ask : undefined;
  const episode = 'origin_ask_episode' in context ? context.origin_ask_episode : undefined;
  const recentlyClosed = 'recently_closed' in context && context.recently_closed;
  const selector = useMemo<ThreadCollectionContext | undefined>(() => {
    if (agent) return { agent_id: agent };
    if (mission) return { mission_id: mission };
    if (ask && episode) return { origin_ask: ask, origin_ask_episode: episode };
    if (recentlyClosed) return { recently_closed: true };
    return undefined;
  }, [agent, mission, ask, episode, recentlyClosed]);
  const read = useCallback((cursor: string | undefined, signal: AbortSignal) => {
    if (!selector) throw new Error('The thread context is incomplete.');
    return client.threadList({ ...selector, cursor, limit: THREAD_WINDOW_SIZE }, { signal });
  }, [client, selector]);
  const follow = useCallback((handlers: ThreadWindowHandlers<ThreadSummary>) => {
    if (selector && feed?.belongsTo(client)) return feed.followThreadSummaries(selector, handlers);
  }, [client, feed, selector]);
  const page = useThreadPage<ThreadSummary>(read, threadRefresh, follow);
  const threads = page.items.filter(thread => thread.id !== excludeId);
  const [show, setShow] = useState(false);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const { height } = useWindowDimensions();
  const scroll = useRef<ScrollView>(null);
  const scrollContent = useRef<View>(null);
  const viewportHeight = useRef(0);
  const offset = useRef(0);
  const attachScroll = useCallback((view: ScrollView | null) => {
    scroll.current = view;
    viewportHeight.current = 0;
    offset.current = 0;
  }, []);
  const revealComposer = useCallback((row: View) => {
    const content = scrollContent.current;
    if (!content) return;
    row.measureLayout(content, (_x, y, _width, rowHeight) => {
      const top = Math.max(0, y - 8);
      const bottom = y + rowHeight + 8 - viewportHeight.current;
      const target = Math.max(bottom, Math.min(top, offset.current));
      if (viewportHeight.current && target !== offset.current) scroll.current?.scrollTo({ y: target, animated: true });
    });
  }, []);
  const { palette } = useTextAppearance();
  if (!threads.length && !page.issue && !page.more) return null;
  return <View style={{ backgroundColor: palette.mantle }}>
    <Pressable accessibilityRole="button" accessibilityState={{ expanded: show }} onPress={() => setShow(previous => !previous)}
      style={{ flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 12, paddingVertical: 6 }}>
      <T numberOfLines={1} style={{ flex: 1 }}><T bold color={theme.accent}>{title}</T>{threads[0] ? <T dim> · {threads[0].title}</T> : null}</T>
      <T dim>{show ? '▾ hide' : '▸ show'}</T>
    </Pressable>
    {show ? <ScrollView ref={attachScroll} innerViewRef={scrollContent as React.RefObject<View>} keyboardShouldPersistTaps="handled" scrollEventThrottle={16}
      onLayout={event => { viewportHeight.current = event.nativeEvent.layout.height; }}
      onScroll={event => { offset.current = event.nativeEvent.contentOffset.y; }}
      style={{ maxHeight: Math.round(height * 0.4), flexGrow: 0 }} contentContainerStyle={{ paddingHorizontal: 12, paddingBottom: 8 }}>
      {threads.map(thread => {
        const open = expanded.has(thread.id);
        return <View key={thread.id}>
          <Pressable
            accessibilityRole="button"
            accessibilityState={{ expanded: open }}
            accessibilityLabel={`${thread.title}, ${threadSummaryLine(thread)}`}
            onPress={() => setExpanded(previous => {
              const next = new Set(previous);
              if (next.has(thread.id)) next.delete(thread.id);
              else next.add(thread.id);
              return next;
            })}
            style={{ minHeight: 36, flexDirection: 'row', alignItems: 'center', gap: 6 }}
          >
            <T numberOfLines={1} color={theme.accent} style={{ flexShrink: 1 }}>{open ? '▾' : '▸'} {thread.title}</T>
            <T dim numberOfLines={1}>· {threadSummaryLine(thread)}</T>
          </Pressable>
          {open ? <ThreadBody client={client} thread={thread} onOpen={() => openThread(thread.id)} revealComposer={revealComposer} /> : null}
        </View>;
      })}
      {page.issue ? <T color={theme.waiting}>{page.issue}</T> : null}
      <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
        <Button label="Refresh threads" onPress={page.reload} disabled={page.loading} />
        {page.more && !page.issue ? <Button label="More threads" onPress={page.more} disabled={page.loading} /> : null}
      </View>
    </ScrollView> : null}
  </View>;
}

export function ThreadScreen({ route }: RootScreen<'Thread'>) {
  const { threadClient } = useStore();
  return <SchemeScreen>
    {threadClient ? <ThreadDetail client={threadClient} id={route.params.id} /> : <T dim style={{ padding: 12 }}>Threads are unavailable on this connection.</T>}
  </SchemeScreen>;
}

function ThreadDetail({ client, id }: { client: ThreadClient; id: string }) {
  const { threadRefresh } = useStore();
  return <ScrollView contentInsetAdjustmentBehavior="automatic" keyboardShouldPersistTaps="handled" contentContainerStyle={{ padding: 12, gap: 8 }}>
    <InlineThread client={client} id={id} refresh={threadRefresh} />
  </ScrollView>;
}

function ThreadDetailContent({ thread, issue, loading, reload, client, onOpen, onTimelineChange }: {
  thread?: ThreadSummary; issue?: string; loading: boolean; reload: () => void; client: ThreadClient; onOpen?: () => void; onTimelineChange: () => void;
}) {
  const [expanded, setExpanded] = useState(true);
  if (thread) return <>
    {onOpen ? <EmbeddedThreadHeader title={thread.title} state={threadSummaryLine(thread)} expanded={expanded} toggle={() => setExpanded(value => !value)} />
      : <T bold>{thread.title}<T dim> · {threadSummaryLine(thread)}</T></T>}
    {expanded || !onOpen ? <ThreadBody client={client} thread={thread} onOpen={onOpen} onTimelineChange={onTimelineChange} /> : null}
  </>;
  if (issue) return <>
    <T color={theme.waiting}>{issue}</T>
    <Button label="Refresh thread" onPress={reload} disabled={loading} />
  </>;
  return <T dim>Loading thread…</T>;
}

function threadSummaryLine(thread: ThreadSummary): string {
  const count = thread.message_count;
  return [
    count === undefined ? undefined : `${count} message${count === 1 ? '' : 's'}`,
    thread.state,
    thread.unread ? 'unread' : undefined,
  ].filter(Boolean).join(' · ');
}

function EmbeddedThreadHeader({ title, state, expanded, toggle }: {
  title: string; state: string; expanded: boolean; toggle: () => void;
}) {
  return <Pressable accessibilityRole="button" accessibilityState={{ expanded }} onPress={toggle}
    style={{ minHeight: 36, flexDirection: 'row', alignItems: 'center', gap: 6 }}>
    <T bold color={theme.accent} numberOfLines={1} style={{ flexShrink: 1 }}>{expanded ? '▾' : '▸'} {title}</T>
    <T dim numberOfLines={1}>· {state}</T>
  </Pressable>;
}

export function AskThread({ origin, initialTitle, onOpen, allowStart = true }: {
  origin: ThreadStartOrigin; initialTitle: string; onOpen: (id: string) => void; allowStart?: boolean;
}) {
  const { threadClient, threadActions } = useStore();
  return threadClient
    ? <AskThreadStart key={`${origin.ask}:${origin.episode}`} client={threadClient} origin={origin} initialTitle={initialTitle} onOpen={onOpen} canStart={allowStart && threadActions.has('thread.start')} />
    : null;
}

function AskThreadStart({ client, origin, initialTitle, onOpen, canStart }: {
  client: ThreadClient; origin: ThreadStartOrigin; initialTitle: string; onOpen: (id: string) => void; canStart: boolean;
}) {
  const { actions, status, threadRefresh, threadDeliveries } = useStore();
  const [started, setStarted] = useState<{ client: ThreadClient; id: string }>();
  const [showComposer, setShowComposer] = useState(false);
  const [draftExpanded, setDraftExpanded] = useState(true);
  const [draft, setDraft] = useState('');
  const [sending, setSending] = useState(false);
  const [failure, setFailure] = useState<{ client: ThreadClient; result: ThreadEditResult }>();
  const delivery = threadDeliveries?.get(threadStartScope(origin));
  const feedback = delivery?.retry ? { how: 'unconfirmed' as const, retry: delivery.retry } : failure?.client === client ? failure.result : undefined;
  const startedId = started?.client === client ? started.id : undefined;
  const content = draft.trim();
  const title = threadTitleFromAsk(initialTitle);
  const issue = threadTextIssue(content);
  const enabled = canStart && status === 'online' && !sending && !delivery;
  const submit = async (id: string, send: () => Promise<ThreadEditResult>) => {
    setSending(true);
    try {
      const next = await send();
      if (next.how === 'sent') {
        setStarted({ client, id });
        setShowComposer(false);
        setDraft('');
      }
      setFailure(next.how === 'failed' ? { client, result: next } : undefined);
    } finally {
      setSending(false);
    }
  };
  const start = () => {
    const id = `thread/${Crypto.randomUUID()}`;
    void submit(id, () => actions.startThread(id, origin, title, content));
  };
  return <View style={{ gap: 6 }}>
    <ThreadGroup context={{ origin_ask: origin.ask, origin_ask_episode: origin.episode }} openThread={onOpen} excludeId={startedId} />
    {startedId ? <InlineThread client={client} id={startedId} refresh={threadRefresh} onOpen={() => onOpen(startedId)} /> : null}
    {!canStart && !delivery ? null
      : !showComposer && !delivery ? <Button label={startedId ? 'Start another thread' : 'Start thread'} disabled={status !== 'online'} onPress={() => {
        setDraftExpanded(true);
        setShowComposer(true);
      }} />
      : <View style={{ gap: 6 }}>
        <EmbeddedThreadHeader title={title} state="draft" expanded={draftExpanded} toggle={() => setDraftExpanded(value => !value)} />
        {draftExpanded ? <>
          <ThreadComposeField text={draft} onChange={setDraft} onSend={start} onCancel={() => setShowComposer(false)}
            enabled={enabled} canSend={!!content && !issue} autoFocus={enabled} />
          {issue ? <T color={theme.waiting}>{issue}</T> : null}
          <ThreadActionFeedback result={feedback} busy={sending || status !== 'online'} onRetry={() => {
            if (delivery?.retry) void submit(delivery.target, delivery.retry);
          }} />
        </> : null}
      </View>}
  </View>;
}

function InlineThread({ client, id, refresh, onOpen }: { client: ThreadClient; id: string; refresh: number; onOpen?: () => void }) {
  const read = useCallback(async (_cursor: string | undefined, signal: AbortSignal) => {
    const { value } = await client.threadGet(id, { signal });
    return { value: { items: [value], next_cursor: null } };
  }, [client, id]);
  const [timelineRevision, bumpTimelineRevision] = useState(0);
  const onTimelineChange = useCallback(() => bumpTimelineRevision(previous => previous + 1), []);
  const page = useThreadPage<ThreadSummary>(read, refresh + timelineRevision);
  return <ThreadDetailContent thread={page.items[0]} issue={page.issue} loading={page.loading} reload={page.reload} client={client} onOpen={onOpen}
    onTimelineChange={onTimelineChange} />;
}

function threadTextIssue(content: string): string | undefined {
  if (content.startsWith('doc/')) return 'Use message text here.';
  if (utf8Bytes(content) > INLINE_MAX_BYTES) return `Keep the message under ${INLINE_MAX_BYTES.toLocaleString()} UTF-8 bytes.`;
}

function ThreadActionFeedback({ result, busy, onRetry }: { result?: ThreadEditResult; busy: boolean; onRetry: () => void }) {
  if (result?.how === 'failed') return <T color={theme.waiting}>{result.error}</T>;
  if (result?.how !== 'unconfirmed') return null;
  return <View style={{ gap: 6 }}>
    <T color={theme.waiting}>The original connection has not confirmed this change.</T>
    <Button label="Retry original change" disabled={busy} onPress={onRetry} />
  </View>;
}

type ThreadMessage = Extract<ThreadTimelineEvent, { kind: 'message' }>;

function eventLine(event: Exclude<ThreadTimelineEvent, ThreadMessage>): string {
  switch (event.kind) {
    case 'thread.renamed': return `renamed “${event.previous_title}” to “${event.title}”`;
    case 'thread.closed': return 'closed the thread';
    case 'thread.reopened': return 'reopened the thread';
    default: return 'updated the thread';
  }
}

// The same thread view renders inside a conversation and on its own screen.
function ThreadBody({ client, thread, onOpen, onTimelineChange, revealComposer }: {
  client: ThreadClient; thread: ThreadSummary; onOpen?: () => void; onTimelineChange?: () => void; revealComposer?: (row: View) => void;
}) {
  const { actions, threadActions, status, threadRefresh, threadDeliveries, actor, data, feed } = useStore();
  const read = useCallback((cursor: string | undefined, signal: AbortSignal) => client.threadTimeline(thread.id, { cursor, limit: THREAD_WINDOW_SIZE }, { signal }), [client, thread.id]);
  const follow = useCallback((handlers: ThreadWindowHandlers<ThreadTimelineEvent>) =>
    feed?.belongsTo(client) ? feed.followThreadTimeline(thread.id, handlers) : undefined, [client, feed, thread.id]);
  const page = useThreadPage<ThreadTimelineEvent>(read, threadRefresh, follow, onTimelineChange);
  const [quote, setQuote] = useState<ThreadMessage>();
  const [quoteFocus, setQuoteFocus] = useState(0);
  const actorLabel = (id: string) => {
    if (actor === id || actor?.startsWith(`${id}/`)) return 'You';
    const agent = data.agents.find(candidate => candidate.id === id);
    return agent ? agentName(agent) : id;
  };
  const messages = new Map(page.items.flatMap(event => event.kind === 'message' ? [[event.id, event] as const] : []));
  const canReply = thread.state === 'open' && threadActions.has('thread.reply');
  const chooseQuote = (message: ThreadMessage) => {
    setQuote(message);
    setQuoteFocus(previous => previous + 1);
  };
  const [editing, setEditing] = useState(false);
  const [editResult, setEditResult] = useState<ThreadEditResult>();
  const editable = status === 'online' && !editing && editResult?.how !== 'unconfirmed';
  const runEdit = async (send: () => Promise<ThreadEditResult>) => {
    setEditing(true);
    try { setEditResult(await send()); }
    finally { setEditing(false); }
  };
  const rename = () => Alert.prompt('Rename thread', undefined, title => {
    const trimmed = title.trim();
    if (trimmed && trimmed !== thread.title) void runEdit(() => actions.editThread(thread, { kind: 'rename', title: trimmed }));
  }, 'plain-text', thread.title);

  return <View style={{ gap: 6, paddingBottom: 8 }}>
    {page.more && !page.issue ? <Button label="Earlier history" onPress={page.more} disabled={page.loading} /> : null}
    {[...page.items].reverse().map(event => {
      const name = actorLabel(event.actor_id);
      const age = ago(new Date(event.accepted_at_unix_ms).toISOString());
      if (event.kind !== 'message') return <T key={event.id} dim><T bold>{name}</T> {eventLine(event)} · {age} ago</T>;
      const quoted = event.thread_quote_id ? messages.get(event.thread_quote_id) : undefined;
      const reply = canReply ? () => chooseQuote(event) : undefined;
      return <Pressable key={event.id} onLongPress={reply} accessible={!!reply}
        accessibilityActions={reply ? [{ name: 'reply', label: 'Reply to this message' }] : undefined}
        onAccessibilityAction={reply ? action => { if (action.nativeEvent.actionName === 'reply') reply(); } : undefined}>
        <T bold>{name}<T dim> · {age} ago</T></T>
        {event.thread_quote_id ? <T dim numberOfLines={1}>{quoted ? `Reply to ${actorLabel(quoted.actor_id)} · ${quoted.content}` : 'Reply to an earlier message'}</T> : null}
        <Markdown text={event.content} selectable={!reply} />
      </Pressable>;
    })}
    {page.issue ? <T color={theme.waiting}>{page.issue}</T> : !page.items.length ? <T dim>{page.loading ? 'Loading history…' : 'No thread messages yet.'}</T> : null}
    {canReply || threadDeliveries?.has(thread.id) ? <ThreadComposer key={thread.id} thread={thread} canReply={canReply}
      selection={quote ? { id: quote.id, from: actorLabel(quote.actor_id), text: quote.content } : undefined} selectionVersion={quoteFocus} revealComposer={revealComposer}
    /> : null}
    {thread.state === 'closed' && threadActions.has('thread.reopen') ? <T dim>Reopen to reply.</T> : null}
    <ThreadActionFeedback result={editResult} busy={editing || status !== 'online'} onRetry={() => {
      if (editResult?.how === 'unconfirmed') void runEdit(editResult.retry);
    }} />
    <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
      {threadActions.has('thread.rename') ? <Button label="Rename" onPress={rename} disabled={!editable} /> : null}
      {threadActions.has(thread.state === 'open' ? 'thread.close' : 'thread.reopen') ? <Button label={thread.state === 'open' ? 'Close thread' : 'Reopen thread'} disabled={!editable} onPress={() => void runEdit(() => actions.editThread(thread, { kind: thread.state === 'open' ? 'close' : 'reopen' }))} /> : null}
      {onOpen ? <Button label="Open thread" onPress={onOpen} /> : null}
      <Button label="Refresh history" onPress={page.reload} disabled={page.loading || status !== 'online'} />
    </View>
  </View>;
}

type Quote = { id: string; from: string; text: string };
type ThreadDraft = { text: string; quote?: Quote };

function readThreadDraft(raw: string | undefined): ThreadDraft {
  if (!raw) return { text: '' };
  try {
    const draft = JSON.parse(raw);
    const quote = draft?.quote;
    return typeof draft?.text === 'string'
      ? { text: draft.text, quote: typeof quote?.id === 'string' && typeof quote.from === 'string' && typeof quote.text === 'string' ? quote : undefined }
      : { text: '' };
  } catch {
    return { text: '' };
  }
}

function ThreadComposer({ thread, canReply, selection, selectionVersion, revealComposer }: {
  thread: ThreadSummary; canReply: boolean; selection?: Quote; selectionVersion: number; revealComposer?: (row: View) => void;
}) {
  const { actions, draftCache, draftsReady, saveDrafts, status, threadDeliveries } = useStore();
  const input = useRef<TextInput>(null);
  const row = useRef<View>(null);
  const reveal = () => {
    if (row.current && input.current?.isFocused()) revealComposer?.(row.current);
  };
  const key = threadDraftKey(thread.id);
  const [draft, setDraft] = useState(() => readThreadDraft(draftCache.current.get(key)));
  const touched = useRef(false);
  const appliedSelection = useRef(selectionVersion);
  const [sending, setSending] = useState(false);
  const [failure, setFailure] = useState<ThreadEditResult>();
  const delivery = threadDeliveries?.get(thread.id);
  const feedback = delivery?.retry ? { how: 'unconfirmed' as const, retry: delivery.retry } : failure;
  const enabled = canReply && status === 'online' && !sending && !delivery;
  const content = draft.text.trim();
  const issue = threadTextIssue(content);

  function storeDraft(next: ThreadDraft) {
    touched.current = true;
    setDraft(next);
    if (next.text || next.quote) rememberBounded(draftCache.current, key, JSON.stringify(next), 24);
    else draftCache.current.delete(key);
  }

  useEffect(() => { if (draftsReady) saveDrafts(); }, [draft, draftsReady]);
  useFocusEffect(useCallback(() => {
    if (draftsReady) setDraft(readThreadDraft(draftCache.current.get(key)));
  }, [draftsReady, draftCache, key]));
  const wasDelivering = useRef(false);
  useEffect(() => {
    const completed = wasDelivering.current && !delivery;
    wasDelivering.current = !!delivery;
    if (draftsReady && (completed || !touched.current)) setDraft(readThreadDraft(draftCache.current.get(key)));
  }, [draftsReady, draftCache, key, delivery]);
  useEffect(() => {
    if (!selection || !enabled || selectionVersion === appliedSelection.current) return;
    appliedSelection.current = selectionVersion;
    storeDraft({ ...draft, quote: selection });
    input.current?.focus();
    reveal();
    // A selection event, rather than a new render, replaces the draft's quote.
  }, [selectionVersion]);

  const send = async (run: () => Promise<ThreadEditResult>) => {
    setSending(true);
    try {
      const next = await run();
      if (next.how === 'sent') setDraft({ text: '' });
      setFailure(next.how === 'failed' ? next : undefined);
    } finally {
      setSending(false);
    }
  };

  return <View style={{ gap: 6 }}>
    {canReply && draft.quote ? <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
      <T dim numberOfLines={1} style={{ flex: 1 }}>Replying to {draft.quote.from} · {draft.quote.text}</T>
      <Button label="Remove quote" disabled={!enabled} onPress={() => storeDraft({ text: draft.text })} />
    </View> : null}
    {canReply ? <ThreadComposeField inputRef={input} rowRef={revealComposer ? row : undefined} onReveal={revealComposer ? reveal : undefined} text={draft.text} onChange={text => storeDraft({ ...draft, text })}
      onSend={() => void send(() => actions.replyToThread(thread, content, draft.quote?.id))} enabled={enabled} canSend={!!content && !issue} /> : null}
    {canReply && issue ? <T color={theme.waiting}>{issue}</T> : null}
    <ThreadActionFeedback result={feedback} busy={sending || status !== 'online'} onRetry={() => {
      if (delivery?.retry) void send(delivery.retry);
    }} />
  </View>;
}

function ThreadComposeField({ inputRef, rowRef, onReveal, text, onChange, onSend, onCancel, enabled, canSend, autoFocus }: {
  inputRef?: Ref<TextInput>; rowRef?: Ref<View>; onReveal?: () => void; text: string; onChange: (text: string) => void; onSend: () => void;
  onCancel?: () => void; enabled: boolean; canSend: boolean; autoFocus?: boolean;
}) {
  return <View ref={rowRef} onLayout={onReveal} style={{ flexDirection: 'row', alignItems: 'flex-end', gap: 6 }}>
    <Field ref={inputRef} onFocus={onReveal} accessibilityLabel="Message thread" placeholder="Message thread" value={text} editable={enabled}
      onChangeText={onChange} multiline autoFocus={autoFocus} style={{ flex: 1 }} />
    <Button label="Send" disabled={!enabled || !canSend} onPress={onSend} />
    {onCancel ? <Button label="Cancel" disabled={!enabled} onPress={onCancel} /> : null}
  </View>;
}
