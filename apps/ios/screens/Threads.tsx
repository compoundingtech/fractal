import { useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactElement, type ReactNode, type Ref } from 'react';
import * as Crypto from 'expo-crypto';
import { AccessibilityInfo, Keyboard, KeyboardAvoidingView, Pressable, ScrollView, StyleSheet, TextInput, useWindowDimensions, View } from 'react-native';
import { useFocusEffect, useNavigation } from '@react-navigation/native';
import { BottomTabBarHeightContext } from '@react-navigation/bottom-tabs';
import { useHeaderHeight } from '@react-navigation/elements';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { ago, INLINE_MAX_BYTES, threadTitleFromAsk, utf8Bytes } from '@fractal/views';
import { ClientError, type CollectionWindow, type Snapshot, type ThreadCollectionContext, type ThreadSummary, type ThreadTimelineEvent } from '../../../clients/typescript/fractal-client';
import { THREAD_WINDOW_SIZE, type ThreadWindowHandlers } from '../feed';
import { useRefresh } from '../chrome';
import { rememberBounded } from '../boundedCache';
import { agentName } from '../agentsView';
import { ContextMenu } from '../menu';
import type { RootScreen } from '../navigation';
import { threadReadErrorText as errorText, threadEditScope, threadDraftKey, threadStartScope, useStore, type ThreadStartOrigin } from '../store';
import type { ThreadEditResult } from '../threadEdits';
import { visibleThreadMessages } from '../threadVisibility';
import { measureThreadBounds, measureThreadViewport, ThreadViewport, useThreadViewports } from '../threadViewport';
import { theme } from '../theme';
import { Button, Field, Markdown, SystemScreen, SystemText, T, useSystemColors, useTextAppearance } from '../ui';

type ThreadPage<T> = { items: T[]; next_cursor: string | null };
type ReadPage<T> = (cursor: string | undefined, signal: AbortSignal) => Promise<{ value: ThreadPage<T>; snapshot?: Snapshot }>;
type FollowPage<T extends { id: string }> = (handlers: ThreadWindowHandlers<T>) => { close(): void } | undefined;

function sameHead<T extends { id: string }>(left: readonly T[], right: readonly T[]): boolean {
  return left.length === right.length && left.every((item, index) => item.id === right[index]?.id);
}

// The live window owns the head; HTTP owns continuation cursors. Never show an
// older tail unless its first page still aligns with the live head.
function useThreadPage<T extends { id: string }>(read: ReadPage<T>, refresh: number, follow?: FollowPage<T>) {
  const active = useRef<AbortController | null>(null);
  const [result, setResult] = useState<{ read: ReadPage<T>; head: T[]; page: ThreadPage<T>; snapshots?: ReadonlyMap<string, Snapshot>; issue?: string }>();
  const [live, setLive] = useState<{ read: ReadPage<T>; follow: FollowPage<T>; window?: CollectionWindow<T>; snapshots?: ReadonlyMap<string, Snapshot>; issue?: string; refused?: boolean }>();
  const [loading, setLoading] = useState(true);
  const visible = result?.read === read ? result : undefined;
  const visibleLive = live?.read === read && live.follow === follow ? live : undefined;
  const load = useCallback(async (prior?: { head: T[]; page: ThreadPage<T>; snapshots?: ReadonlyMap<string, Snapshot> }) => {
    active.current?.abort();
    const request = new AbortController();
    active.current = request;
    setLoading(true);
    try {
      const { value, snapshot } = await read(prior?.page.next_cursor ?? undefined, request.signal);
      if (!request.signal.aborted) {
        const page = prior ? { ...value, items: [...prior.page.items, ...value.items] } : value;
        const snapshots = new Map(prior?.snapshots);
        if (snapshot) for (const item of value.items) snapshots.set(item.id, snapshot);
        setResult({ read, head: prior?.head ?? value.items, page, snapshots });
        setLive(previous => previous?.read === read ? { ...previous, issue: undefined, refused: false } : previous);
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
    // A released (collapsed, blurred, unmounted or re-keyed) follow must never
    // write into this reader again, even if a frame was already in flight.
    let released = false;
    let current: CollectionWindow<T> | undefined;
    const held = follow({
      onWindow: window => {
        if (released) return;
        const changed = !current || !sameHead(current.items, window.items) || current.hasMore !== window.hasMore;
        current = window;
        setLive({ read, follow, window });
        if (changed) void load();
      },
      onIssue: (issue, refused) => {
        if (released) return;
        if (refused) {
          active.current?.abort();
          setResult(undefined);
          setLoading(false);
        }
        setLive({ read, follow, issue, refused, window: refused ? undefined : current });
      },
    });
    return () => {
      released = true;
      held?.close();
    };
  }, [follow, load, read]));

  // Exact detail streams can refresh a cached continuation row without adding
  // it to the context's live head or changing its ordering/pagination boundary.
  const replace = useCallback((item: T, snapshot?: Snapshot) => {
    if (!snapshot) return;
    const update = (items: T[]) => items.map(previous => previous.id === item.id ? item : previous);
    setResult(previous => {
      if (previous?.read !== read || !previous.page.items.some(row => row.id === item.id)) return previous;
      const seen = previous.snapshots?.get(item.id);
      if (seen && (seen.host_id !== snapshot.host_id || seen.store_index > snapshot.store_index)) return previous;
      return { ...previous, page: { ...previous.page, items: update(previous.page.items) }, snapshots: new Map(previous.snapshots).set(item.id, snapshot) };
    });
    setLive(previous => {
      if (previous?.read !== read || !previous.window?.items.some(row => row.id === item.id)) return previous;
      const seen = previous.snapshots?.get(item.id) ?? previous.window.snapshot;
      if (seen.host_id !== snapshot.host_id || seen.store_index > snapshot.store_index) return previous;
      return { ...previous, window: { ...previous.window, items: update(previous.window.items) }, snapshots: new Map(previous.snapshots).set(item.id, snapshot) };
    });
  }, [read]);

  const liveWindow = visibleLive?.window;
  const aligned = !liveWindow || !!visible && sameHead(visible.head, liveWindow.items);
  const tail = aligned && visible && liveWindow ? visible.page.items.slice(visible.head.length) : [];
  const items = visibleLive?.refused ? [] : liveWindow ? [...liveWindow.items, ...tail] : visible?.page.items ?? [];
  // Continuation rows retain their own read snapshot; a newer head cannot certify an older tail.
  const snapshots = useMemo(() => {
    const known = new Map(visible?.snapshots);
    if (liveWindow) for (const item of liveWindow.items) known.set(item.id, visibleLive?.snapshots?.get(item.id) ?? liveWindow.snapshot);
    return known;
  }, [visible?.snapshots, liveWindow, visibleLive?.snapshots]);
  return {
    // A followed window updates itself; manual refresh is only for HTTP-only readers.
    live: !!liveWindow,
    items,
    snapshots,
    issue: visibleLive?.issue || visible?.issue,
    loading,
    replace,
    reload: () => void load(),
    more: aligned && !visibleLive?.refused && visible?.page.next_cursor ? () => void load(visible) : undefined,
  };
}

type ThreadClient = NonNullable<ReturnType<typeof useStore>['threadClient']>;
/** Opens the standalone thread screen; the title names it before its summary loads. */
type OpenThread = (id: string, title?: string) => void;
/** Lays out a thread's scrolling history and the part docked below it (closed hint, composer). */
type ThreadFrame = (content: ReactNode, dock?: ReactNode) => ReactElement;

/** Enlarged text wraps titles instead of truncating them to an unreadable sliver. */
const ACCESSIBILITY_FONT_SCALE = 1.3;
function useTitleLines(): number | undefined {
  return useWindowDimensions().fontScale > ACCESSIBILITY_FONT_SCALE ? undefined : 1;
}

export function AgentThreads({ agentId, openThread }: { agentId: string; openThread: OpenThread }) {
  return <ThreadGroup context={{ agent_id: agentId }} openThread={openThread} />;
}

export function ThreadGroup({ context, openThread, title = 'Threads', excludeId }: {
  context: ThreadCollectionContext; openThread: OpenThread; title?: string; excludeId?: string;
}) {
  const { threadClient } = useStore();
  // Embedded thread groups read in system text on every host, as the thread screen does.
  return threadClient ? <SystemText><ContextThreadList client={threadClient} context={context} openThread={openThread} title={title} excludeId={excludeId} /></SystemText> : null;
}

function ContextThreadList({ context, client, openThread, title, excludeId }: {
  context: ThreadCollectionContext; client: ThreadClient; openThread: OpenThread; title: string; excludeId?: string;
}) {
  const { threadRefresh, feed, threadEdits } = useStore();
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
  useEffect(() => { page.items.forEach(thread => threadEdits?.observe(thread, page.snapshots.get(thread.id))); }, [page.items, page.snapshots, threadEdits, threadRefresh]);
  const threads = page.items.filter(thread => thread.id !== excludeId);
  const [show, setShow] = useState(false);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const { height } = useWindowDimensions();
  const titleLines = useTitleLines();
  // A host that scrolls (mission, Inbox) lets the group grow taller; above a conversation it
  // keeps half the screen for the conversation. The group's own scroll keeps the composer reveal.
  const hosted = useThreadViewports().length > 0;
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
  const count = `${threads.length}${page.more ? '+' : ''}`;
  return <View style={{ backgroundColor: appearance.system ? appearance.colors.grouped : appearance.palette.mantle }}>
    <Pressable accessibilityRole="button" accessibilityState={{ expanded: show }} onPress={() => setShow(previous => !previous)}
      accessibilityLabel={`${title}, ${threads.length}${page.more ? ' or more' : ''}`}
      style={{ minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 12 }}>
      <T numberOfLines={titleLines} style={{ flex: 1 }}><T bold color={theme.accent}>{title}</T><T dim> · {count}</T></T>
      <T dim>{show ? 'Hide' : 'Show'}</T>
    </Pressable>
    {show ? <ScrollView ref={attachScroll} innerViewRef={scrollContent as React.RefObject<View>} keyboardShouldPersistTaps="handled" scrollEventThrottle={16}
      onLayout={event => { viewportHeight.current = event.nativeEvent.layout.height; }}
      onScroll={event => { offset.current = event.nativeEvent.contentOffset.y; }}
      style={{ maxHeight: Math.round(height * (hosted ? 0.75 : 0.5)), flexGrow: 0 }} contentContainerStyle={{ paddingHorizontal: 12, paddingBottom: 8 }}>
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
          onSummary={page.replace} toggle={toggle} onOpen={() => openThread(thread.id, (threadEdits?.project(thread) ?? thread).title)} revealComposer={revealComposer} />
          : <ContextThreadHeader key={thread.id} thread={thread} open={false} toggle={toggle} />;
      })}
      </ThreadViewport>
      {page.issue ? <><T color={theme.waiting}>{page.issue}</T><Button label="Retry" onPress={page.reload} disabled={page.loading} /></> : null}
      {!page.live || page.more && !page.issue ? <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
        {!page.live ? <Button label="Refresh threads" onPress={page.reload} disabled={page.loading} /> : null}
        {page.more && !page.issue ? <Button label="More threads" onPress={page.more} disabled={page.loading} /> : null}
      </View> : null}
    </ScrollView> : null}
  </View>;
}

function ContextThreadHeader({ thread, open, toggle }: { thread: ThreadSummary; open: boolean; toggle: () => void }) {
  const { threadEdits } = useStore();
  const displayed = threadEdits?.project(thread) ?? thread;
  return <ThreadRow title={displayed.title} label={`${displayed.title}, ${threadSummaryLine(displayed, ', ')}`}
    unread={displayed.unread} trailing={displayed.state === 'closed' ? 'Closed' : undefined} expanded={open} toggle={toggle} />;
}

function ExpandedContextThread({ client, initial, refresh, onSummary, toggle, onOpen, revealComposer }: {
  client: ThreadClient; initial: ThreadSummary; refresh: number; onSummary: (thread: ThreadSummary, snapshot?: Snapshot) => void; toggle: () => void; onOpen: () => void; revealComposer: (row: View) => void;
}) {
  // The context window only follows its head. An expanded older page needs the
  // same exact summary stream as a standalone thread, including its action fence.
  const page = useThreadSummary(client, initial.id, refresh, true, onSummary);
  const thread = page.items[0];
  return <View>
    <ContextThreadHeader thread={thread ?? initial} open toggle={toggle} />
    {thread ? <ThreadBody client={client} thread={thread} onOpen={onOpen} revealComposer={revealComposer} />
      : page.issue ? <><T color={theme.waiting}>{page.issue}</T><Button label="Retry" onPress={page.reload} disabled={page.loading} /></>
      : <T dim>Loading thread…</T>}
  </View>;
}

export function ThreadScreen({ route }: RootScreen<'Thread'>) {
  const { threadClient } = useStore();
  return <SystemScreen>
    {threadClient ? <ThreadDetail client={threadClient} id={route.params.id} /> : <T dim style={{ padding: 12 }}>Threads are unavailable on this connection.</T>}
  </SystemScreen>;
}

// The history scrolls in the space left above a docked composer, as in Messages; the keyboard
// lifts the dock, and the viewport that proves which replies were seen ends where the dock begins.
function ThreadDetail({ client, id }: { client: ThreadClient; id: string }) {
  const { threadRefresh } = useStore();
  const viewport = useRef<ScrollView>(null);
  const refresh = useRefresh();
  const headerHeight = useHeaderHeight();
  const insets = useSafeAreaInsets();
  const colors = useSystemColors();
  // Above the keyboard the dock needs no home-indicator inset.
  const [keyboard, setKeyboard] = useState(false);
  useEffect(() => {
    const shown = Keyboard.addListener('keyboardWillShow', () => setKeyboard(true));
    const hidden = Keyboard.addListener('keyboardWillHide', () => setKeyboard(false));
    return () => { shown.remove(); hidden.remove(); };
  }, []);
  const frame: ThreadFrame = (content, dock) => <KeyboardAvoidingView style={{ flex: 1 }} behavior="padding" keyboardVerticalOffset={headerHeight}>
    <ScrollView ref={viewport} style={{ flex: 1 }} contentInsetAdjustmentBehavior="automatic" keyboardShouldPersistTaps="handled" keyboardDismissMode="interactive"
      refreshControl={refresh} contentContainerStyle={{ padding: 12, gap: 8 }}>{content}</ScrollView>
    {dock ? <View style={{ paddingHorizontal: 12, paddingTop: 8, paddingBottom: keyboard ? 8 : Math.max(insets.bottom, 8), borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: colors.separator, backgroundColor: colors.background }}>
      {dock}
    </View> : null}
  </KeyboardAvoidingView>;
  return <ThreadViewport viewport={viewport}><InlineThread client={client} id={id} refresh={threadRefresh} frame={frame} /></ThreadViewport>;
}

function ThreadDetailContent({ thread, issue, loading, reload, client, onOpen, start, embedded, frame }: {
  thread?: ThreadSummary; issue?: string; loading: boolean; reload: () => void; client: ThreadClient; onOpen?: () => void; start?: ThreadStartDraft; embedded?: boolean; frame?: ThreadFrame;
}) {
  const [expanded, setExpanded] = useState(true);
  const { threadEdits } = useStore();
  const navigation = useNavigation();
  const displayed = thread && (threadEdits?.project(thread) ?? thread);
  const title = displayed?.title ?? start?.title;
  const standalone = !(onOpen || embedded);
  // The standalone screen names the thread in its navigation bar, following renames.
  useEffect(() => { if (standalone && title) navigation.setOptions({ title }); }, [navigation, standalone, title]);
  if (thread || start) {
    const pending = start?.accepted ? 'Loading history…' : 'draft';
    const summary = standalone ? <T dim>{displayed ? threadSummaryLine(displayed, ' · ', displayed.state !== 'closed') : pending}</T>
      : <ThreadRow bold title={title!} label={`${title}, ${displayed ? threadSummaryLine(displayed, ', ') : pending}`} unread={displayed?.unread}
        trailing={displayed ? displayed.state === 'closed' ? 'Closed' : undefined : pending} expanded={expanded} toggle={() => setExpanded(value => !value)} />;
    // A framed (docked) thread puts its summary line at the top of the scrolling history.
    if (frame) return <ThreadBody client={client} thread={thread} start={start} onOpen={onOpen} frame={(content, dock) => frame(<>{summary}{content}</>, dock)} />;
    return <>
      {summary}
      {expanded || standalone ? <ThreadBody client={client} thread={thread} start={start} onOpen={onOpen} /> : null}
    </>;
  }
  const status = issue ? <>
    <T color={theme.waiting}>{issue}</T>
    <Button label="Refresh thread" onPress={reload} disabled={loading} />
  </> : <T dim>Loading thread…</T>;
  return frame ? frame(status) : status;
}

function threadSummaryLine(thread: ThreadSummary, separator = ' · ', includeState = true): string {
  const count = thread.message_count;
  return [
    count === undefined ? undefined : `${count} message${count === 1 ? '' : 's'}`,
    includeState ? thread.state : undefined,
    thread.unread ? 'unread' : undefined,
  ].filter(Boolean).join(separator);
}

/** One thread row: an unread dot, the title, and Closed. Counts and state stay in the spoken label. */
function ThreadRow({ title, label, unread, trailing, expanded, toggle, bold }: {
  title: string; label: string; unread?: boolean; trailing?: string; expanded: boolean; toggle: () => void; bold?: boolean;
}) {
  const appearance = useTextAppearance();
  const titleLines = useTitleLines();
  return <Pressable accessibilityRole="button" accessibilityState={{ expanded }} accessibilityLabel={label} onPress={toggle}
    style={{ minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 8 }}>
    <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: unread ? appearance.color(theme.accent) : 'transparent' }} />
    <T bold={bold} color={theme.accent} numberOfLines={titleLines} style={{ flex: 1 }}>{title}</T>
    {trailing ? <T dim numberOfLines={1}>{trailing}</T> : null}
  </Pressable>;
}

export function InboxThread({ sourceId, origin, agentId, initialTitle, onOpen, pendingScope }: {
  sourceId: string; origin?: Pick<ThreadStartOrigin, 'sourceId' | 'episode'>; agentId?: string;
  initialTitle: string; onOpen: OpenThread; pendingScope?: string;
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

export function PendingInboxThreads({ onOpen }: { onOpen: OpenThread }) {
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
  initialTitle: string; onOpen: OpenThread; canStart: boolean; showHistory?: boolean;
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
      onOpen={target && accepted ? () => onOpen(target, draft.title) : undefined} start={draft} embedded /> : null}
    {!agentId ? <T dim>No agent is assigned to this Inbox item.</T> : null}
    {canStart && (!draft || accepted) ? <Button label={accepted ? 'Start another thread' : 'Start thread'} disabled={status !== 'online' || !!delivery}
      onPress={() => setLocal(previous => ({ client, accepted: false, generation: (previous?.generation ?? 0) + 1 }))} /> : null}
  </View>;
}

function InlineThread({ client, id, refresh, onOpen, start, embedded, frame }: {
  client: ThreadClient; id?: string; refresh: number; onOpen?: () => void; start?: ThreadStartDraft; embedded?: boolean; frame?: ThreadFrame;
}) {
  const page = useThreadSummary(client, id, refresh, !start || start.accepted);
  return <ThreadDetailContent thread={page.items[0]} issue={page.issue} loading={page.loading} reload={page.reload} client={client} onOpen={onOpen} start={start} embedded={embedded} frame={frame} />;
}

function useThreadSummary(client: ThreadClient, id: string | undefined, refresh: number, accepted = true, onSummary?: (thread: ThreadSummary, snapshot?: Snapshot) => void) {
  const { feed, threadEdits } = useStore();
  const read = useCallback(async (_cursor: string | undefined, signal: AbortSignal) => {
    if (!accepted || !id) return { value: { items: [], next_cursor: null } };
    const { value, snapshot } = await client.threadGet(id, { signal });
    return { value: { items: [value], next_cursor: null }, snapshot };
  }, [accepted, client, id]);
  const follow = useCallback((handlers: ThreadWindowHandlers<ThreadSummary>) =>
    accepted && id && feed?.belongsTo(client) ? feed.followThreadSummary(id, handlers) : undefined, [accepted, client, feed, id]);
  const page = useThreadPage<ThreadSummary>(read, refresh, follow);
  // Hand the cache exactly the summary this reader shows, whether it came from
  // REST or the live stream, and only after commit: an aborted read or a frame
  // for a released follow never renders here, so it can never reach the cache.
  const thread = page.items[0];
  const snapshot = thread && page.snapshots?.get(thread.id);
  useEffect(() => {
    if (thread) { threadEdits?.observe(thread, snapshot); onSummary?.(thread, snapshot); }
  }, [onSummary, thread, threadEdits, snapshot, refresh]);
  return page;
}

function threadTextIssue(content: string): string | undefined {
  if (content.startsWith('doc/')) return 'Use message text here.';
  if (utf8Bytes(content) > INLINE_MAX_BYTES) return `Keep the message under ${INLINE_MAX_BYTES.toLocaleString()} UTF-8 bytes.`;
}

function ThreadActionFeedback({ result, busy, onRetry }: { result?: ThreadEditResult; busy: boolean; onRetry: () => void }) {
  if (result?.how === 'failed') return <View style={{ gap: 6 }}>
    {result.action ? <T color={theme.waiting}>{`Couldn't ${result.action} this thread.`}</T> : null}
    <T color={theme.waiting}>{result.error}</T>
  </View>;
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

function ThreadEditFlash({ message }: { message: string }) {
  useEffect(() => { AccessibilityInfo.announceForAccessibility(message); }, [message]);
  return <View accessible accessibilityLiveRegion="polite" accessibilityLabel={message} style={{ paddingVertical: 6 }}>
    <T color={theme.accent}>{message}</T>
  </View>;
}

// The same thread view renders inside a conversation and on its own screen.
function ThreadBody({ client, thread, start, onOpen, revealComposer, frame }: {
  client: ThreadClient; thread?: ThreadSummary; start?: ThreadStartDraft; onOpen?: () => void; revealComposer?: (row: View) => void; frame?: ThreadFrame;
}) {
  const { actions, threadActions, status, threadRefresh, threadDeliveries, threadEdits, actor, data, feed } = useStore();
  const displayed = thread && (threadEdits?.project(thread) ?? thread);
  const flash = thread && threadEdits?.flash(thread.id);
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
  // Replies stay open on a closed thread; Reopen is a separate, explicit change.
  const closed = displayed?.state === 'closed';
  // Rows on the timeline's latest calendar day read as clock times so their order is legible;
  // earlier days keep their age. A latest day other than today is named once above its rows.
  const timeline = [...page.items].reverse();
  const latest = timeline.reduce<number | undefined>((max, event) => Math.max(max ?? event.accepted_at_unix_ms, event.accepted_at_unix_ms), undefined);
  const latestDay = latest === undefined ? undefined : new Date(latest).toDateString();
  const firstOfLatestDay = timeline.find(event => new Date(event.accepted_at_unix_ms).toDateString() === latestDay)?.id;
  const dayHeading = latest !== undefined && latestDay !== new Date().toDateString()
    ? new Date(latest).toLocaleDateString(undefined, { month: 'short', day: 'numeric' }) : undefined;
  const when = (acceptedAt: number) => {
    const at = new Date(acceptedAt);
    if (at.toDateString() !== latestDay) return `${ago(at.toISOString())} ago`;
    const pad = (value: number) => String(value).padStart(2, '0');
    return `${pad(at.getHours())}:${pad(at.getMinutes())}`;
  };
  const chooseQuote = (message: ThreadMessage) => {
    setQuote(message);
    setQuoteFocus(previous => previous + 1);
  };
  const [editing, setEditing] = useState(false);
  const [editResult, setEditResult] = useState<ThreadEditResult>();
  const [renameTitle, setRenameTitle] = useState<string>();
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
  const rename = () => {
    const title = renameTitle?.trim();
    if (!thread || !title || title === displayed?.title || !editable) return;
    setRenameTitle(undefined);
    void runEdit(() => actions.editThread(thread, { kind: 'rename', title }), true);
  };
  const changeState = () => {
    if (thread) void runEdit(() => actions.editThread(thread, { kind: closed ? 'reopen' : 'close' }), true);
  };

  const earlier = page.more && !page.issue ? <Button label="Earlier history" onPress={page.more} disabled={page.loading} /> : null;
  const rows = timeline.flatMap(event => {
      const name = actorLabel(event.actor_id);
      const age = when(event.accepted_at_unix_ms);
      const heading = dayHeading && event.id === firstOfLatestDay ? <T key={`day:${event.id}`} dim>{dayHeading}</T> : null;
      if (event.kind !== 'message') return [heading, <T key={event.id} dim><T bold>{name}</T> {eventLine(event)} · {age}</T>];
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
        <T bold>{name}<T dim> · {age}</T></T>
        {event.thread_quote_id ? <T dim numberOfLines={1}>{quoted ? `Reply to ${actorLabel(quoted.actor_id)} · ${quoted.content}` : 'Reply to an earlier message'}</T> : null}
        <Markdown text={event.content} selectable={!reply} />
      </Pressable>;
      return [heading, <ContextMenu key={event.id} actions={readHere ? [
        ...(reply ? [{ id: 'reply', title: 'Reply to this message', run: reply }] : []),
        { id: 'markRead', title: 'Mark visible replies read', run: markVisibleRead },
      ] : []}>{row}</ContextMenu>];
  });
  const history = page.issue ? <><T color={theme.waiting}>{page.issue}</T><Button label="Retry" onPress={page.reload} disabled={page.loading} /></> : !start && !page.items.length ? <T dim>{page.loading ? 'Loading history…' : 'No thread messages yet.'}</T> : null;
  const markRead = readCandidates.length ? <Button label="Mark visible replies read" disabled={!editable} onPress={markVisibleRead} /> : null;
  const closedHint = closed ? <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
    <View accessible accessibilityRole="text" accessibilityLabel={canReply ? 'Closed, replying keeps it closed' : 'Closed'} style={{ flex: 1 }}>
      <T dim>{canReply ? 'Closed · replying keeps it closed' : 'Closed'}</T>
    </View>
    {threadActions.has('thread.reopen') ? <Button label="Reopen thread" disabled={!editable} onPress={changeState} /> : null}
  </View> : null;
  const composer = canReply || id && threadDeliveries?.has(id) || start ? <ThreadComposer key={start?.cacheKey ?? id} thread={thread} start={start} canReply={canReply}
    selection={quote ? { id: quote.id, from: actorLabel(quote.actor_id), text: quote.content } : undefined} selectionVersion={quoteFocus} revealComposer={revealComposer}
  /> : null;
  const unavailable = thread && !canReply ? <T dim>Replies aren't available on this connection.</T> : null;
  const flashLine = flash ? <ThreadEditFlash message={flash} /> : null;
  const renameField = renameTitle !== undefined ? <View style={{ gap: 6 }}>
    <Field accessibilityLabel="Thread title" autoFocus value={renameTitle} onChangeText={setRenameTitle}
      editable={editable} returnKeyType="done" onSubmitEditing={rename} />
    <View style={{ flexDirection: 'row', gap: 6 }}>
      <Button label="Save title" onPress={rename} disabled={!editable || !renameTitle.trim() || renameTitle.trim() === displayed?.title} />
      <Button label="Cancel rename" onPress={() => setRenameTitle(undefined)} />
    </View>
  </View> : null;
  const editStatus = <ThreadActionFeedback result={editFeedback} busy={editing || !!editDelivery && !editDelivery.retry || status !== 'online'} onRetry={() => {
    if (editFeedback?.how === 'unconfirmed') void runEdit(editFeedback.retry, !!editDelivery);
  }} />;
  const threadButtons = thread ? <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
    {threadActions.has('thread.rename') ? <Button label="Rename" onPress={() => setRenameTitle(displayed!.title)} disabled={!editable} /> : null}
    {!closed && threadActions.has('thread.close') ? <Button label="Close thread" disabled={!editable} onPress={changeState} /> : null}
    {onOpen ? <Button label="Open thread" onPress={onOpen} /> : null}
    {!page.live ? <Button label="Refresh history" onPress={page.reload} disabled={page.loading || status !== 'online'} /> : null}
  </View> : null;

  // The thread screen docks the confirmation, closed hint and the one composer below its scrolling
  // history; embedded threads keep the same elements inline, in reading order.
  if (frame) return frame(<View style={{ gap: 6 }}>{earlier}{rows}{history}{markRead}{renameField}{editStatus}{threadButtons}</View>,
    flashLine || closedHint || composer || unavailable ? <View style={{ gap: 6 }}>{flashLine}{closedHint}{composer}{unavailable}</View> : undefined);
  return <View style={{ gap: 6, paddingBottom: 8 }}>
    {earlier}{rows}{history}{markRead}{closedHint}{composer}{unavailable}{flashLine}{renameField}{editStatus}{threadButtons}
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
    {/* iOS appends the placeholder to the label only while empty; keep one name in both states. */}
    <Field ref={inputRef} onFocus={onReveal} accessibilityLabel={text.length ? 'Message thread' : undefined} placeholder="Message thread" value={text} editable={enabled}
      onChangeText={onChange} multiline autoFocus={autoFocus} style={{ flex: 1 }} />
    <Button label="Send" disabled={!enabled || !canSend} onPress={onSend} />
    {onCancel ? <Button label="Cancel" disabled={!enabled} onPress={onCancel} /> : null}
  </View>;
}
