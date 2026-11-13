import { useCallback, useContext, useEffect, useMemo, useRef, useState, type Ref } from 'react';
import * as Crypto from 'expo-crypto';
import { Alert, Pressable, ScrollView, TextInput, useWindowDimensions, View } from 'react-native';
import { useFocusEffect, useNavigation } from '@react-navigation/native';
import { BottomTabBarHeightContext } from '@react-navigation/bottom-tabs';
import { useHeaderHeight } from '@react-navigation/elements';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { ago, INLINE_MAX_BYTES, threadTitleFromAsk, utf8Bytes } from '@fractal/views';
import { ClientError, type CollectionWindow, type ThreadCollectionContext, type ThreadSummary, type ThreadTimelineEvent } from '../../../clients/typescript/fractal-client';
import { THREAD_WINDOW_SIZE, type ThreadWindowHandlers } from '../feed';
import { rememberBounded } from '../boundedCache';
import { agentName } from '../agentsView';
import { ContextMenu } from '../menu';
import type { RootScreen } from '../navigation';
import { errorText, threadEditScope, threadDraftKey, threadStartScope, useStore, type ThreadEditResult, type ThreadStartOrigin } from '../store';
import { visibleThreadMessages } from '../threadVisibility';
import { measureThreadBounds, measureThreadViewport, ThreadViewport, useThreadViewports } from '../threadViewport';
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
function useThreadPage<T extends { id: string }>(read: ReadPage<T>, refresh: number, follow?: FollowPage<T>) {
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
  }, [follow, load, read]));

  // Exact detail streams can refresh a cached continuation row without adding
  // it to the context's live head or changing its ordering/pagination boundary.
  const replace = useCallback((item: T) => {
    const update = (items: T[]) => items.map(previous => previous.id === item.id ? item : previous);
    setResult(previous => previous?.read === read ? { ...previous, page: { ...previous.page, items: update(previous.page.items) } } : previous);
    setLive(previous => previous?.read === read && previous.window ? { ...previous, window: { ...previous.window, items: update(previous.window.items) } } : previous);
  }, [read]);

  const liveWindow = visibleLive?.window;
  const aligned = !liveWindow || !!visible && sameHead(visible.head, liveWindow.items);
  const tail = aligned && visible && liveWindow ? visible.page.items.slice(visible.head.length) : [];
  const items = visibleLive?.refused ? [] : liveWindow ? [...liveWindow.items, ...tail] : visible?.page.items ?? [];
  return {
    items,
    issue: visibleLive?.issue || visible?.issue,
    loading,
    replace,
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
  const ask = 'origin_source' in context ? context.origin_source : undefined;
  const episode = 'origin_episode' in context ? context.origin_episode : undefined;
  const recentlyClosed = 'recently_closed' in context && context.recently_closed;
  const selector = useMemo<ThreadCollectionContext | undefined>(() => {
    if (agent) return { agent_id: agent };
    if (mission) return { mission_id: mission };
    if (ask && episode) return { origin_source: ask, origin_episode: episode };
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
  const appearance = useTextAppearance();
  if (!threads.length && !page.issue && !page.more) return null;
  return <View style={{ backgroundColor: appearance.system ? appearance.colors.grouped : appearance.palette.mantle }}>
    <Pressable accessibilityRole="button" accessibilityState={{ expanded: show }} onPress={() => setShow(previous => !previous)}
      style={{ flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 12, paddingVertical: 6 }}>
      <T numberOfLines={1} style={{ flex: 1 }}><T bold color={theme.accent}>{title}</T>{threads[0] ? <T dim> · {threads[0].title}</T> : null}</T>
      <T dim>{show ? '▾ hide' : '▸ show'}</T>
    </Pressable>
    {show ? <ScrollView ref={attachScroll} innerViewRef={scrollContent as React.RefObject<View>} keyboardShouldPersistTaps="handled" scrollEventThrottle={16}
      onLayout={event => { viewportHeight.current = event.nativeEvent.layout.height; }}
      onScroll={event => { offset.current = event.nativeEvent.contentOffset.y; }}
      style={{ maxHeight: Math.round(height * 0.4), flexGrow: 0 }} contentContainerStyle={{ paddingHorizontal: 12, paddingBottom: 8 }}>
      <ThreadViewport viewport={scroll}>
      {threads.map(thread => {
        const open = expanded.has(thread.id);
        const toggle = () => setExpanded(previous => {
          const next = new Set(previous);
          if (next.has(thread.id)) next.delete(thread.id);
          else next.add(thread.id);
          return next;
        });
        return open ? <ExpandedContextThread key={thread.id} client={client} initial={thread} refresh={threadRefresh}
          onSummary={page.replace} toggle={toggle} onOpen={() => openThread(thread.id)} revealComposer={revealComposer} />
          : <ContextThreadHeader key={thread.id} thread={thread} open={false} toggle={toggle} />;
      })}
      </ThreadViewport>
      {page.issue ? <T color={theme.waiting}>{page.issue}</T> : null}
      <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
        <Button label="Refresh threads" onPress={page.reload} disabled={page.loading} />
        {page.more && !page.issue ? <Button label="More threads" onPress={page.more} disabled={page.loading} /> : null}
      </View>
    </ScrollView> : null}
  </View>;
}

function ContextThreadHeader({ thread, open, toggle }: { thread: ThreadSummary; open: boolean; toggle: () => void }) {
  return <Pressable accessibilityRole="button" accessibilityState={{ expanded: open }}
    accessibilityLabel={`${thread.title}, ${threadSummaryLine(thread)}`} onPress={toggle}
    style={{ minHeight: 36, flexDirection: 'row', alignItems: 'center', gap: 6 }}>
    <T numberOfLines={1} color={theme.accent} style={{ flexShrink: 1 }}>{open ? '▾' : '▸'} {thread.title}</T>
    <T dim numberOfLines={1}>· {threadSummaryLine(thread)}</T>
  </Pressable>;
}

function ExpandedContextThread({ client, initial, refresh, onSummary, toggle, onOpen, revealComposer }: {
  client: ThreadClient; initial: ThreadSummary; refresh: number; onSummary: (thread: ThreadSummary) => void; toggle: () => void; onOpen: () => void; revealComposer: (row: View) => void;
}) {
  // The context window only follows its head. An expanded older page needs the
  // same exact summary stream as a standalone thread, including its action fence.
  const page = useThreadSummary(client, initial.id, refresh, true, onSummary);
  const thread = page.items[0];
  return <View>
    <ContextThreadHeader thread={thread ?? initial} open toggle={toggle} />
    {thread ? <ThreadBody client={client} thread={thread} onOpen={onOpen} revealComposer={revealComposer} />
      : page.issue ? <><T color={theme.waiting}>{page.issue}</T><Button label="Refresh thread" onPress={page.reload} disabled={page.loading} /></>
      : <T dim>Loading thread…</T>}
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
  const viewport = useRef<ScrollView>(null);
  return <ScrollView ref={viewport} contentInsetAdjustmentBehavior="automatic" automaticallyAdjustKeyboardInsets keyboardShouldPersistTaps="handled" contentContainerStyle={{ padding: 12, gap: 8 }}>
    <ThreadViewport viewport={viewport}><InlineThread client={client} id={id} refresh={threadRefresh} /></ThreadViewport>
  </ScrollView>;
}

function ThreadDetailContent({ thread, issue, loading, reload, client, onOpen, start, embedded }: {
  thread?: ThreadSummary; issue?: string; loading: boolean; reload: () => void; client: ThreadClient; onOpen?: () => void; start?: ThreadStartDraft; embedded?: boolean;
}) {
  const [expanded, setExpanded] = useState(true);
  if (thread || start) return <>
    {onOpen || embedded ? <EmbeddedThreadHeader title={thread?.title ?? start!.title} state={thread ? threadSummaryLine(thread) : start?.accepted ? 'Loading history…' : 'draft'} expanded={expanded} toggle={() => setExpanded(value => !value)} />
      : <T bold>{thread?.title ?? start!.title}<T dim> · {thread ? threadSummaryLine(thread) : start?.accepted ? 'Loading history…' : 'draft'}</T></T>}
    {expanded || !(onOpen || embedded) ? <ThreadBody client={client} thread={thread} start={start} onOpen={onOpen} /> : null}
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

export function InboxThread({ sourceId, origin, agentId, initialTitle, onOpen, pendingScope }: {
  sourceId: string; origin?: Pick<ThreadStartOrigin, 'sourceId' | 'episode'>; agentId?: string;
  initialTitle: string; onOpen: (id: string) => void; pendingScope?: string;
}) {
  const { threadClient, threadActions, threadDeliveries } = useStore();
  if (!threadClient) return null;
  const retained = [...(threadDeliveries?.values() ?? [])].find(delivery => delivery.start
    && delivery.start.origin.sourceId === sourceId && (!pendingScope || threadStartScope(delivery.start.origin) === pendingScope));
  const shownOrigin = retained?.start?.origin ?? origin;
  if (!shownOrigin) return <T dim>Thread context is unavailable for this Inbox item.</T>;
  return <InboxThreadStart key={threadStartScope(shownOrigin)} client={threadClient}
    origin={shownOrigin} agentId={retained?.start?.origin.agentId ?? agentId}
    initialTitle={retained?.start?.title ?? initialTitle} onOpen={onOpen} showHistory={!!origin}
    canStart={!retained && !!origin && !!agentId && threadActions.has('thread.start')} />;
}

export function PendingInboxThreads({ onOpen }: { onOpen: (id: string) => void }) {
  const { threadClient, threadDeliveries, data } = useStore();
  if (!threadClient) return null;
  const listed = new Set(data.attention.map(item => item.source_id));
  const seen = new Set<string>();
  const pending = [...(threadDeliveries?.values() ?? [])].flatMap(delivery => {
    const start = delivery.start;
    if (!start || listed.has(start.origin.sourceId)) return [];
    const scope = threadStartScope(start.origin);
    if (seen.has(scope)) return [];
    seen.add(scope);
    return [start];
  });
  if (!pending.length) return null;
  return <View style={{ padding: 12, gap: 6 }}>
    <T bold>Unconfirmed thread starts</T>
    {pending.map(start => <View key={threadStartScope(start.origin)} style={{ gap: 4 }}>
      <T dim>Original recipient · {start.origin.agentId}</T>
      <InboxThread sourceId={start.origin.sourceId} pendingScope={threadStartScope(start.origin)} initialTitle={start.title} onOpen={onOpen} />
    </View>)}
  </View>;
}

type ThreadStartDraft = {
  target?: string; cacheKey: string; origin: ThreadStartOrigin; title: string; accepted: boolean;
  onAccepted: (id: string) => void; onCancel: () => void;
};

function InboxThreadStart({ client, origin, agentId, initialTitle, onOpen, canStart, showHistory = true }: {
  client: ThreadClient; origin: Pick<ThreadStartOrigin, 'sourceId' | 'episode'>; agentId?: string;
  initialTitle: string; onOpen: (id: string) => void; canStart: boolean; showHistory?: boolean;
}) {
  const { status, threadRefresh, threadDeliveries, draftCache, draftsReady, saveDrafts } = useStore();
  const scope = threadStartScope(origin);
  const cacheKey = threadDraftKey(scope);
  const [local, setLocal] = useState<{ client: ThreadClient; target?: string; accepted: boolean; generation: number }>();
  const delivery = threadDeliveries?.get(scope);
  const current = local?.client === client ? local : undefined;
  const target = current?.target ?? delivery?.target;
  const accepted = current?.accepted === true;
  const show = !!current || !!delivery || draftsReady && draftCache.current.has(cacheKey);
  const draft: ThreadStartDraft | undefined = show && agentId ? {
    target, cacheKey, origin: { ...origin, agentId }, accepted, title: threadTitleFromAsk(initialTitle),
    onAccepted: id => setLocal({ client, target: id, accepted: true, generation: current?.generation ?? 0 }),
    onCancel: () => {
      draftCache.current.delete(cacheKey);
      saveDrafts();
      setLocal(undefined);
    },
  } : undefined;
  return <View style={{ gap: 6 }}>
    {showHistory ? <ThreadGroup context={{ origin_source: origin.sourceId, origin_episode: origin.episode }} openThread={onOpen} excludeId={accepted ? target : undefined} /> : null}
    {draft ? <InlineThread key={`${scope}:${current?.generation ?? 0}`} client={client} id={target} refresh={threadRefresh}
      onOpen={target && accepted ? () => onOpen(target) : undefined} start={draft} embedded /> : null}
    {!agentId ? <T dim>No agent is assigned to this Inbox item.</T> : null}
    {canStart && (!draft || accepted) ? <Button label={accepted ? 'Start another thread' : 'Start thread'} disabled={status !== 'online' || !!delivery}
      onPress={() => setLocal(previous => ({ client, accepted: false, generation: (previous?.generation ?? 0) + 1 }))} /> : null}
  </View>;
}

function InlineThread({ client, id, refresh, onOpen, start, embedded }: {
  client: ThreadClient; id?: string; refresh: number; onOpen?: () => void; start?: ThreadStartDraft; embedded?: boolean;
}) {
  const page = useThreadSummary(client, id, refresh, !start || start.accepted);
  return <ThreadDetailContent thread={page.items[0]} issue={page.issue} loading={page.loading} reload={page.reload} client={client} onOpen={onOpen} start={start} embedded={embedded} />;
}

function useThreadSummary(client: ThreadClient, id: string | undefined, refresh: number, accepted = true, onSummary?: (thread: ThreadSummary) => void) {
  const { feed } = useStore();
  const read = useCallback(async (_cursor: string | undefined, signal: AbortSignal) => {
    if (!accepted || !id) return { value: { items: [], next_cursor: null } };
    const { value } = await client.threadGet(id, { signal });
    return { value: { items: [value], next_cursor: null } };
  }, [accepted, client, id]);
  const follow = useCallback((handlers: ThreadWindowHandlers<ThreadSummary>) =>
    accepted && id && feed?.belongsTo(client) ? feed.followThreadSummary(id, {
      ...handlers,
      onWindow: window => {
        handlers.onWindow(window);
        if (window.items[0]) onSummary?.(window.items[0]);
      },
    }) : undefined, [accepted, client, feed, id, onSummary]);
  return useThreadPage<ThreadSummary>(read, refresh, follow);
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
function ThreadBody({ client, thread, start, onOpen, revealComposer }: {
  client: ThreadClient; thread?: ThreadSummary; start?: ThreadStartDraft; onOpen?: () => void; revealComposer?: (row: View) => void;
}) {
  const { actions, threadActions, status, threadRefresh, threadDeliveries, actor, data, feed } = useStore();
  const id = thread?.id ?? start?.target;
  const read = useCallback(async (cursor: string | undefined, signal: AbortSignal) => thread
    ? client.threadTimeline(thread.id, { cursor, limit: THREAD_WINDOW_SIZE }, { signal })
    : { value: { items: [], next_cursor: null } }, [client, id, !!thread]);
  const follow = useCallback((handlers: ThreadWindowHandlers<ThreadTimelineEvent>) =>
    thread && feed?.belongsTo(client) ? feed.followThreadTimeline(thread.id, handlers) : undefined, [client, feed, id, !!thread]);
  const page = useThreadPage<ThreadTimelineEvent>(read, threadRefresh, follow);
  const { width, height } = useWindowDimensions();
  const insets = useSafeAreaInsets();
  const headerHeight = useHeaderHeight();
  const tabBarHeight = useContext(BottomTabBarHeightContext) ?? 0;
  const navigation = useNavigation();
  const viewports = useThreadViewports();
  const messageViews = useRef(new Map<string, View>());
  const [quote, setQuote] = useState<ThreadMessage>();
  const [quoteFocus, setQuoteFocus] = useState(0);
  const ownMessage = (id: string) => actor === id || actor?.startsWith(`${id}/`);
  const actorLabel = (id: string) => {
    if (ownMessage(id)) return 'You';
    const agent = data.agents.find(candidate => candidate.id === id);
    return agent ? agentName(agent) : id;
  };
  const messages = new Map(page.items.flatMap(event => event.kind === 'message' ? [[event.id, event] as const] : []));
  const canReply = threadActions.has(thread ? 'thread.reply' : 'thread.start');
  const chooseQuote = (message: ThreadMessage) => {
    setQuote(message);
    setQuoteFocus(previous => previous + 1);
  };
  const [editing, setEditing] = useState(false);
  const [editResult, setEditResult] = useState<ThreadEditResult>();
  const editDelivery = thread ? threadDeliveries?.get(threadEditScope(thread.id)) : undefined;
  const editFeedback = editDelivery?.retry ? { how: 'unconfirmed' as const, retry: editDelivery.retry } : editResult;
  const editable = status === 'online' && !editing && !editDelivery && editFeedback?.how !== 'unconfirmed';
  const runEdit = async (send: () => Promise<ThreadEditResult>, retained = false) => {
    setEditing(true);
    setEditResult(undefined);
    try { const result = await send(); setEditResult(retained && result.how === 'unconfirmed' ? undefined : result); }
    finally { setEditing(false); }
  };
  const canMarkRead = thread?.unread === true && !!actor && threadActions.has('thread.read');
  const readCandidates = canMarkRead ? page.items.filter((event): event is ThreadMessage =>
    event.kind === 'message' && !ownMessage(event.actor_id)) : [];
  const markVisibleRead = () => void runEdit(async () => {
    if (!thread) return { how: 'failed', error: 'This thread has not been sent yet.' };
    const top = Math.max(headerHeight, insets.top);
    const [viewport, rows] = await Promise.all([
      measureThreadViewport(viewports, { x: insets.left, y: top, width: width - insets.left - insets.right, height: height - top - Math.max(tabBarHeight, insets.bottom) }),
      Promise.all(readCandidates.map(async ({ id }) => ({ id, bounds: await measureThreadBounds(messageViews.current.get(id)) }))),
    ]);
    if (!navigation.isFocused() || !viewport) return { how: 'failed', error: 'The visible replies could not be verified.' };
    const ids = visibleThreadMessages(viewport, rows, THREAD_WINDOW_SIZE);
    if (!ids.length) return { how: 'failed', error: 'Scroll a reply into view to mark it read.' };
    return actions.markThreadRead(thread, ids);
  });
  const rename = () => thread && Alert.prompt('Rename thread', undefined, title => {
    const trimmed = title.trim();
    if (trimmed && trimmed !== thread.title) void runEdit(() => actions.editThread(thread, { kind: 'rename', title: trimmed }), true);
  }, 'plain-text', thread.title);

  return <View style={{ gap: 6, paddingBottom: 8 }}>
    {page.more && !page.issue ? <Button label="Earlier history" onPress={page.more} disabled={page.loading} /> : null}
    {[...page.items].reverse().map(event => {
      const name = actorLabel(event.actor_id);
      const age = ago(new Date(event.accepted_at_unix_ms).toISOString());
      if (event.kind !== 'message') return <T key={event.id} dim><T bold>{name}</T> {eventLine(event)} · {age} ago</T>;
      const quoted = event.thread_quote_id ? messages.get(event.thread_quote_id) : undefined;
      const reply = canReply ? () => chooseQuote(event) : undefined;
      const readHere = canMarkRead && !ownMessage(event.actor_id) && editable;
      const row = <Pressable key={event.id} onLongPress={reply} accessible={!!reply || readHere}
        collapsable={canMarkRead ? false : undefined}
        ref={canMarkRead ? view => {
          if (view) messageViews.current.set(event.id, view);
          else messageViews.current.delete(event.id);
        } : undefined}
        accessibilityActions={[
          ...(reply ? [{ name: 'reply', label: 'Reply to this message' }] : []),
          ...(readHere ? [{ name: 'markRead', label: 'Mark visible replies read' }] : []),
        ]}
        onAccessibilityAction={action => {
          if (action.nativeEvent.actionName === 'reply') reply?.();
          if (action.nativeEvent.actionName === 'markRead' && readHere) markVisibleRead();
        }}>
        <T bold>{name}<T dim> · {age} ago</T></T>
        {event.thread_quote_id ? <T dim numberOfLines={1}>{quoted ? `Reply to ${actorLabel(quoted.actor_id)} · ${quoted.content}` : 'Reply to an earlier message'}</T> : null}
        <Markdown text={event.content} selectable={!reply} />
      </Pressable>;
      return <ContextMenu key={event.id} actions={readHere ? [
        ...(reply ? [{ id: 'reply', title: 'Reply to this message', run: reply }] : []),
        { id: 'markRead', title: 'Mark visible replies read', run: markVisibleRead },
      ] : []}>{row}</ContextMenu>;
    })}
    {page.issue ? <T color={theme.waiting}>{page.issue}</T> : !start && !page.items.length ? <T dim>{page.loading ? 'Loading history…' : 'No thread messages yet.'}</T> : null}
    {readCandidates.length ? <Button label="Mark visible replies read" disabled={!editable} onPress={markVisibleRead} /> : null}
    {canReply || id && threadDeliveries?.has(id) || start ? <ThreadComposer key={start?.cacheKey ?? id} thread={thread} start={start} canReply={canReply}
      selection={quote ? { id: quote.id, from: actorLabel(quote.actor_id), text: quote.content } : undefined} selectionVersion={quoteFocus} revealComposer={revealComposer}
    /> : null}
    <ThreadActionFeedback result={editFeedback} busy={editing || !!editDelivery && !editDelivery.retry || status !== 'online'} onRetry={() => {
      if (editFeedback?.how === 'unconfirmed') void runEdit(editFeedback.retry, !!editDelivery);
    }} />
    {thread ? <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
      {threadActions.has('thread.rename') ? <Button label="Rename" onPress={rename} disabled={!editable} /> : null}
      {threadActions.has(thread.state === 'open' ? 'thread.close' : 'thread.reopen') ? <Button label={thread.state === 'open' ? 'Close thread' : 'Reopen thread'} disabled={!editable} onPress={() => void runEdit(() => actions.editThread(thread, { kind: thread.state === 'open' ? 'close' : 'reopen' }), true)} /> : null}
      {onOpen ? <Button label="Open thread" onPress={onOpen} /> : null}
      <Button label="Refresh history" onPress={page.reload} disabled={page.loading || status !== 'online'} />
    </View> : null}
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

function ThreadComposer({ thread, start, canReply, selection, selectionVersion, revealComposer }: {
  thread?: ThreadSummary; start?: ThreadStartDraft; canReply: boolean; selection?: Quote; selectionVersion: number; revealComposer?: (row: View) => void;
}) {
  const { actions, draftCache, draftsReady, saveDrafts, status, threadDeliveries } = useStore();
  const input = useRef<TextInput>(null);
  const row = useRef<View>(null);
  const reveal = () => {
    if (row.current && input.current?.isFocused()) revealComposer?.(row.current);
  };
  const id = thread?.id ?? start?.target;
  const key = thread || start?.accepted ? threadDraftKey(id!) : start!.cacheKey;
  const [draft, setDraft] = useState(() => readThreadDraft(draftCache.current.get(key)));
  const touched = useRef(false);
  const appliedSelection = useRef(selectionVersion);
  const [sending, setSending] = useState(false);
  const sentTarget = useRef<string | undefined>(undefined);
  const [failure, setFailure] = useState<ThreadEditResult>();
  const delivery = threadDeliveries?.get(!thread && start ? threadStartScope(start.origin) : id!);
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
      if (next.how === 'sent') {
        storeDraft({ text: '' });
        if (start && !thread) {
          start.onAccepted(sentTarget.current!);
        }
      }
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
      onSend={() => void send(() => {
        if (thread) return actions.replyToThread(thread, content, draft.quote?.id);
        const target = start!.target ?? `thread/${Crypto.randomUUID()}`;
        sentTarget.current = target;
        return actions.startThread(target, start!.origin, start!.title, content);
      })} enabled={enabled} canSend={(!!thread || !start?.accepted) && !!content && !issue}
      autoFocus={!!start && !thread} onCancel={start && !thread && !start.accepted && !delivery && !sending ? start.onCancel : undefined} /> : null}
    {canReply && issue ? <T color={theme.waiting}>{issue}</T> : null}
    <ThreadActionFeedback result={feedback} busy={sending || status !== 'online'} onRetry={() => {
      if (delivery?.retry) { sentTarget.current = delivery.target; void send(delivery.retry); }
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
