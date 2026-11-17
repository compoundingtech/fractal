import { ConversationComposer } from '../conversationComposer';
import { ConversationFeedback, ConversationFrame, ConversationMessage } from '../conversationPresentation';
import { useCallback, useContext, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactElement, type ReactNode, type Ref } from 'react';
import * as Crypto from 'expo-crypto';
import * as Clipboard from 'expo-clipboard';
import { AccessibilityInfo, findNodeHandle, Keyboard, Pressable, ScrollView, StyleSheet, Text, TextInput, useWindowDimensions, View } from 'react-native';
import { useFocusEffect, useNavigation } from '@react-navigation/native';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import { BottomTabBarHeightContext } from '@react-navigation/bottom-tabs';
import { useHeaderHeight } from '@react-navigation/elements';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { INLINE_MAX_BYTES, threadTitleFromAsk, utf8Bytes } from '@fractal/views';
import { ClientError, type CollectionWindow, type RequestHistoryItem, type Snapshot, type ThreadCollectionContext, type ThreadSummary, type ThreadTimelineEvent, type ThreadTimelinePage } from '../../../clients/typescript/fractal-client';
import { THREAD_WINDOW_SIZE, type ThreadWindowHandlers } from '../feed';
import { useRefresh } from '../chrome';
import { agentName } from '../agentsView';
import { ContextMenu } from '../menu';
import type { RootScreen, StackParams } from '../navigation';
import { threadReadErrorText as errorText, threadEditScope, threadDraftKey, threadStartAttemptScope, threadStartScope, threadStartSlot, useStore, type ThreadStartOrigin } from '../store';
import type { ThreadEditResult } from '../threadEdits';
import { ThreadStartContextReader } from '../threadStartContext';
import { savedThreadStarts, startRecoveryKey, type SavedThreadStart } from '../threadStartRecovery';
import { threadClock, threadExactTime, threadRelativeTime } from '../threadClock';
import { visibleThreadMessages, type ThreadBounds } from '../threadVisibility';
import { measureThreadBounds, measureThreadViewport, threadViewportChanged, ThreadViewport, useThreadViewports } from '../threadViewport';
import { anchorPresentation, reminderFromSummary } from '../unreadVisit';
import { useUnreadVisit, useUnreadVisitEntry } from '../unreadVisitNative';
import { pendingUnreadAPI, type UnreadAPI } from '../unreadActions';
import type { UnreadList, UnreadReadError, UnreadSeek } from '../unreadClient';
import { theme } from '../theme';
import { Button, Field, SystemScreen, SystemText, T, useSystemColors, useTextAppearance, type MarkdownLine } from '../ui';
import { RequestHistory } from './RequestHistory';
import { StartRecipientPreview } from './StartRecipientPreview';

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
  const [result, setResult] = useState<{ read: ReadPage<T>; head: T[]; page: ThreadPage<T>; snapshots?: ReadonlyMap<string, Snapshot>; issue?: string; refused?: boolean }>();
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
        const refused = error instanceof ClientError && [401, 403, 404].includes(error.status);
        setResult({ read, head: canKeepPrior && prior ? prior.head : [], page: canKeepPrior && prior ? prior.page : { items: [], next_cursor: null }, issue: errorText(error), refused });
        if (refused) {
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
        // A successful authoritative frame supersedes an earlier HTTP refusal,
        // even if its IDs/hasMore are unchanged and no HTTP reload is needed.
        setResult(previous => previous?.read === read && previous.refused ? undefined : previous);
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
    refused: !!(visibleLive?.refused || visible?.refused),
    loading,
    replace,
    reload: () => void load(),
    more: aligned && !visibleLive?.refused && visible?.page.next_cursor ? () => void load(visible) : undefined,
  };
}

/** Typed read failures name their subject; no failure may read as an empty list. */
function unreadReadIssue(error: UnreadReadError, subject: string): string {
  switch (error.kind) {
    case 'refused': return error.code === 'upgrade-required' ? 'Upgrade the connected Fractal to use unread reminders.'
      : error.code === 'forbidden' ? `This connection can't read ${subject}.` : `The connected Fractal refused to load ${subject} (${error.code}).`;
    case 'invalid-response': return `The connected Fractal returned an invalid response for ${subject}.`;
    case 'failed': return `Couldn't load ${subject}. ${errorText(error.cause)}`;
  }
}

function markedUnreadIssue(error: Extract<UnreadList, { kind: 'error' }>['error']): string {
  switch (error.kind) {
    case 'page-bound': return `More marked threads exist than ${error.maxPages} pages. Only the first pages are shown.`;
    case 'invalid-page-bound': return `Marked threads can't be listed with a ${error.maxPages}-page limit.`;
    case 'repeated-cursor': return 'The connected Fractal repeated a page of marked threads, so this list may be incomplete.';
    case 'read-failed': return unreadReadIssue(error.reason, 'marked threads');
  }
}

const NO_THREADS: ThreadSummary[] = [];
const NO_SNAPSHOTS: ReadonlyMap<string, Snapshot> = new Map();

/** The owner-wide marked list is read whole, within the paging bound, and is not followed.
 * A typed failure keeps the rows already read and is shown with them, never as empty.
 */
function useMarkedUnreadPage(enabled: boolean, refresh: number) {
  const { unreadClient } = useStore();
  const [result, setResult] = useState<{ reader: typeof unreadClient; items: ThreadSummary[]; issue?: string; refused?: boolean }>();
  const [loading, setLoading] = useState(false);
  const request = useRef(0);
  const load = useCallback(async () => {
    if (!enabled) return;
    const token = ++request.current;
    setLoading(true);
    const list = await unreadClient.listMarked();
    if (token !== request.current) return;
    // Only an authoritative summary reminder lists a row; natural unread alone never does.
    const items = list.kind === 'empty' ? [] : list.items.filter(thread => !!reminderFromSummary(thread));
    setResult({ reader: unreadClient, items, issue: list.kind === 'error' ? markedUnreadIssue(list.error) : undefined,
      refused: list.kind === 'error' && list.error.kind === 'read-failed' && list.error.reason.kind === 'refused' });
    setLoading(false);
  }, [enabled, unreadClient]);
  useFocusEffect(useCallback(() => {
    void load();
    // A blurred or released list never applies a list that was still in flight.
    return () => { request.current++; };
  }, [load, refresh]));
  // An expanded row's exact summary stream refreshes its own row, never the membership.
  const replace = useCallback((item: ThreadSummary, _snapshot?: Snapshot) => setResult(previous => previous && {
    ...previous, items: previous.items.map(row => row.id === item.id ? item : row),
  }), []);
  const visible = result?.reader === unreadClient ? result : undefined;
  return {
    live: false, items: visible?.items ?? NO_THREADS, snapshots: NO_SNAPSHOTS, issue: visible?.issue, refused: visible?.refused, loading, replace,
    reload: () => void load(), more: undefined,
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
  const { threadClient, unreadEnabled, actor, draftGeneration } = useStore();
  if ('marked_unread' in context && !unreadEnabled) return null;
  // Embedded thread groups read in system text on every host, as the thread screen does.
  return threadClient ? <SystemText><ContextThreadList key={`${actor}:${draftGeneration.current}`} client={threadClient} context={context} openThread={openThread} title={title} excludeId={excludeId} /></SystemText> : null;
}

function ContextThreadList({ context, client, openThread, title, excludeId }: {
  context: ThreadCollectionContext; client: ThreadClient; openThread: OpenThread; title: string; excludeId?: string;
}) {
  const { threadRefresh, feed, threadEdits, actor, draftGeneration } = useStore();
  const agent = 'agent_id' in context ? context.agent_id : undefined;
  const mission = 'mission_id' in context ? context.mission_id : undefined;
  const ask = 'origin_source' in context ? context.origin_source : undefined;
  const episode = 'origin_episode' in context ? context.origin_episode : undefined;
  const recentlyClosed = 'recently_closed' in context && context.recently_closed;
  const marked = 'marked_unread' in context && context.marked_unread === true;
  const selector = useMemo<ThreadCollectionContext | undefined>(() => {
    if (agent) return { agent_id: agent };
    if (mission) return { mission_id: mission };
    if (ask && episode) return { origin_source: ask, origin_episode: episode };
    if (recentlyClosed) return { recently_closed: true };
    if (marked) return { marked_unread: true };
    return undefined;
  }, [agent, mission, ask, episode, recentlyClosed, marked]);
  // Marked unread reads through the unread client's bounded, typed paging instead.
  const read = useCallback((cursor: string | undefined, signal: AbortSignal) => {
    if (marked) return Promise.resolve({ value: { items: [], next_cursor: null } });
    if (!selector) throw new Error('The thread context is incomplete.');
    return client.threadList({ ...selector, cursor, limit: THREAD_WINDOW_SIZE }, { signal });
  }, [client, marked, selector]);
  const follow = useCallback((handlers: ThreadWindowHandlers<ThreadSummary>) => {
    if (!marked && selector && feed?.belongsTo(client)) return feed.followThreadSummaries(selector, handlers);
  }, [client, feed, marked, selector]);
  const contextPage = useThreadPage<ThreadSummary>(read, threadRefresh, follow);
  const markedPage = useMarkedUnreadPage(marked, threadRefresh);
  const page = marked ? markedPage : contextPage;
  useEffect(() => { page.items.forEach(thread => threadEdits?.observe(thread, page.snapshots.get(thread.id))); }, [page.items, page.snapshots, threadEdits, threadRefresh]);
  const placementScope = JSON.stringify(selector ?? null);
  const [show, setShow] = useState(false);
  const lifetime = JSON.stringify([actor, draftGeneration.current, placementScope]);
  const [opened, setOpened] = useState<{ client: ThreadClient; lifetime: string; items: ReadonlyMap<string, ThreadSummary>; order: readonly string[] }>();
  const owned = opened?.client === client && opened.lifetime === lifetime ? opened : undefined;
  const expanded = owned?.items;
  // Only navigation/ownership exits release the reading placement, never a list refresh.
  useFocusEffect(useCallback(() => () => setOpened(undefined), [client, lifetime]));
  const members = page.items.filter(thread => thread.id !== excludeId && (!marked || !!reminderFromSummary(thread)) && (!recentlyClosed || thread.state === 'closed'));
  const threads = [...members];
  if (expanded) for (const thread of expanded.values()) {
    if (thread.id !== excludeId && !threads.some(row => row.id === thread.id)) threads.push(thread);
  }
  if (owned?.items.size) {
    // Freeze the visit's row ordering: removing an open row from membership must
    // not move its body/composer below the remaining rows in the same ScrollView.
    const positions = new Map(owned.order.map((id, index) => [id, index]));
    threads.sort((left, right) => (positions.get(left.id) ?? owned.order.length) - (positions.get(right.id) ?? owned.order.length));
  }
  const onSummary = useCallback((thread: ThreadSummary, snapshot?: Snapshot) => {
    page.replace(thread, snapshot);
    setOpened(previous => {
      if (previous?.client !== client || previous.lifetime !== lifetime || !previous.items.has(thread.id) || previous.items.get(thread.id) === thread) return previous;
      return { ...previous, items: new Map(previous.items).set(thread.id, thread) };
    });
  }, [page.replace, client, lifetime]);
  useEffect(() => { if (page.refused) setOpened(undefined); }, [page.refused]);
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
  if (page.refused) return <ThreadReadIssue message={page.issue!} />;
  if (!threads.length && !page.issue && !page.more) return null;
  // The marked section adds no new count: each row's private marker is its only signal.
  const count = marked ? undefined : `${members.length}${page.more ? '+' : ''}`;
  return <View style={{ backgroundColor: appearance.system ? appearance.colors.grouped : appearance.palette.mantle }}>
    <Pressable accessibilityRole="button" accessibilityState={{ expanded: show }} onPress={() => { setShow(previous => !previous); if (show) setOpened(undefined); }}
      accessibilityLabel={marked ? title : `${title}, ${members.length}${page.more ? ' or more' : ''}`}
      accessibilityValue={{ text: show ? 'Expanded' : 'Collapsed' }} accessibilityHint={show ? 'Hides the threads' : 'Shows the threads'}
      style={{ minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 12 }}>
      <T numberOfLines={titleLines} style={{ flex: 1 }}><T bold color={theme.accent}>{title}</T>{count ? <T dim> · {count}</T> : null}</T>
      <T dim>{show ? 'Hide' : 'Show'}</T>
    </Pressable>
    {show ? <ScrollView ref={attachScroll} innerViewRef={scrollContent as React.RefObject<View>} keyboardShouldPersistTaps="handled" scrollEventThrottle={16}
      onLayout={event => { viewportHeight.current = event.nativeEvent.layout.height; threadViewportChanged(scroll); }}
      onScroll={event => { offset.current = event.nativeEvent.contentOffset.y; threadViewportChanged(scroll); }}
      style={{ maxHeight: Math.round(height * (hosted ? 0.75 : 0.5)), flexGrow: 0 }} contentContainerStyle={{ paddingHorizontal: 12, paddingBottom: 8 }}>
      <ThreadViewport viewport={scroll}>
      {threads.map(thread => {
        const open = expanded?.has(thread.id) ?? false;
        const toggle = () => setOpened(previous => {
          const previousOwned = previous?.client === client && previous.lifetime === lifetime ? previous : undefined;
          const items = new Map(previousOwned?.items);
          if (items.has(thread.id)) items.delete(thread.id);
          else items.set(thread.id, thread);
          return { client, lifetime, items, order: previousOwned?.items.size ? previousOwned.order : threads.map(row => row.id) };
        });
        return open ? <ExpandedContextThread key={thread.id} client={client} initial={thread} refresh={threadRefresh} placement={`context:${placementScope}:${thread.id}`} seekReminder={marked}
          onSummary={onSummary} toggle={toggle} onOpen={() => openThread(thread.id, (threadEdits?.project(thread) ?? thread).title)} revealComposer={revealComposer} />
          : <ContextThreadHeader key={thread.id} thread={thread} open={false} toggle={toggle} />;
      })}
      </ThreadViewport>
      {page.issue ? <><ThreadReadIssue message={page.issue} /><Button label="Retry" accessibilityLabel="Retry loading threads" onPress={page.reload} disabled={page.loading} /></> : null}
      {!page.live || page.more && !page.issue ? <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
        {!page.live ? <Button label="Refresh threads" onPress={page.reload} disabled={page.loading} /> : null}
        {page.more && !page.issue ? <Button label="More threads" onPress={page.more} disabled={page.loading} /> : null}
      </View> : null}
    </ScrollView> : null}
  </View>;
}

function ContextThreadHeader({ thread, open, toggle }: { thread: ThreadSummary; open: boolean; toggle: () => void }) {
  const { threadEdits, unreadEnabled } = useStore();
  const displayed = threadEdits?.project(thread) ?? thread;
  return <ThreadRow title={displayed.title} label={`${displayed.title}, ${threadSummaryLine(displayed, ', ', true, unreadEnabled)}`}
    unread={displayed.unread} reminder={!!unreadEnabled && !!reminderFromSummary(displayed)} trailing={displayed.state === 'closed' ? 'Closed' : undefined} expanded={open} toggle={toggle} />;
}

function ExpandedContextThread({ client, initial, refresh, placement, seekReminder, onSummary, toggle, onOpen, revealComposer }: {
  client: ThreadClient; initial: ThreadSummary; refresh: number; placement: string; seekReminder?: boolean; onSummary: (thread: ThreadSummary, snapshot?: Snapshot) => void; toggle: () => void; onOpen: () => void; revealComposer: (row: View) => void;
}) {
  // The context window only follows its head. An expanded older page needs the
  // same exact summary stream as a standalone thread, including its action fence.
  const page = useThreadSummary(client, initial.id, refresh, true, onSummary);
  const thread = page.items[0];
  // Expanding is the visit's entry, before the exact summary or body exists. The listed
  // summary is already known, so a reminder it carries is captured at entry.
  useUnreadVisitEntry({ scope: client, placementKey: placement, threadId: initial.id, actor: useStore().actor, summary: thread ?? initial });
  return <View>
    <ContextThreadHeader thread={thread ?? initial} open toggle={toggle} />
    {thread ? seekReminder ? <ReminderThreadBody client={client} thread={thread} placement={placement} onOpen={onOpen} revealComposer={revealComposer} />
      : <ThreadBody client={client} thread={thread} placement={placement} onOpen={onOpen} revealComposer={revealComposer} />
      : page.issue ? <><ThreadReadIssue message={page.issue} /><Button label="Retry" accessibilityLabel={`Retry loading ${initial.title}`} onPress={page.reload} disabled={page.loading} /></>
      : <T dim>Loading thread…</T>}
  </View>;
}

/** A marked thread opens around its private return message, read-only: the seek never
 * marks anything read or cleared. Only a current anchor has a body; unavailable and
 * pending anchors show their notice alone, with no history and no new unread signal,
 * beside an explicit Clear reminder that needs no body.
 */
function ReminderThreadBody({ client, thread, placement, onOpen, revealComposer }: {
  client: ThreadClient; thread: ThreadSummary; placement: string; onOpen: () => void; revealComposer?: (row: View) => void;
}) {
  const { unreadClient, unreadEnabled } = useStore();
  const anchorId = unreadEnabled ? reminderFromSummary(thread)?.anchor_message_id : undefined;
  const [attempt, setAttempt] = useState(0);
  const [seek, setSeek] = useState<{ anchor: string; attempt: number; result?: UnreadSeek }>();
  const latest = useRef(thread);
  latest.current = thread;
  useEffect(() => {
    if (!anchorId) return;
    let current = true;
    setSeek({ anchor: anchorId, attempt });
    const api = unreadClient.forThread(latest.current);
    const seeking = api.state === 'ready' ? api.actions.seek(anchorId)
      : Promise.resolve<UnreadSeek>({ state: 'error', error: { kind: 'refused', code: 'forbidden' } });
    void seeking.then(
      result => { if (current) setSeek({ anchor: anchorId, attempt, result }); },
      cause => { if (current) setSeek({ anchor: anchorId, attempt, result: { state: 'error', error: { kind: 'failed', cause } } }); },
    );
    return () => { current = false; };
  }, [anchorId, attempt, unreadClient]);
  // Clearing the reminder during this visit keeps the page it opened around.
  const target = unreadEnabled ? anchorId ?? seek?.anchor : undefined;
  if (!target) return <ThreadBody client={client} thread={thread} placement={placement} onOpen={onOpen} revealComposer={revealComposer} />;
  const result = seek && seek.anchor === target && seek.attempt === attempt ? seek.result : undefined;
  if (!result) return <View style={{ gap: 6, paddingBottom: 8 }}>
    <T dim>Finding the return message…</T>
    <ClearReminder thread={thread} />
  </View>;
  switch (result.state) {
    case 'current': return <ThreadBody client={client} thread={thread} placement={placement} around={result.page} onOpen={onOpen} revealComposer={revealComposer} />;
    case 'unavailable':
    case 'pending': return <View style={{ gap: 6, paddingBottom: 8 }}>
      <T dim>{anchorPresentation(result.anchor).notice}</T>
      <ClearReminder thread={thread} />
      <Button label="Open thread" hint={`Opens ${thread.title}`} onPress={onOpen} />
    </View>;
    case 'error': return <View style={{ gap: 6, paddingBottom: 8 }}>
      <ThreadReadIssue message={unreadReadIssue(result.error, 'the return message')} />
      <ClearReminder thread={thread} />
      <Button label="Retry" accessibilityLabel={`Retry finding the return message in ${thread.title}`} onPress={() => setAttempt(previous => previous + 1)} />
    </View>;
  }
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
  const frame: ThreadFrame = (content, dock) => ConversationFrame({
    keyboardVerticalOffset: headerHeight,
    children: <ScrollView ref={viewport} style={{ flex: 1 }} contentInsetAdjustmentBehavior="automatic" keyboardShouldPersistTaps="handled" keyboardDismissMode="interactive" refreshControl={refresh}
      onScroll={() => threadViewportChanged(viewport)} scrollEventThrottle={100} onLayout={() => threadViewportChanged(viewport)}
      contentContainerStyle={{ padding: 12, gap: 8 }}>{content}</ScrollView>,
    composer: dock,
    composerStyle: { paddingHorizontal: 12, paddingTop: 8, paddingBottom: keyboard ? 8 : Math.max(insets.bottom, 8), borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: colors.separator, backgroundColor: colors.background },
  });
  return <ThreadViewport viewport={viewport}><InlineThread client={client} id={id} placement={`thread:${id}`} refresh={threadRefresh} frame={frame} /></ThreadViewport>;
}

function ThreadDetailContent({ thread, issue, loading, reload, client, placement, onOpen, start, embedded, frame }: {
  thread?: ThreadSummary; issue?: string; loading: boolean; reload: () => void; client: ThreadClient; placement: string; onOpen?: () => void; start?: ThreadStartDraft; embedded?: boolean; frame?: ThreadFrame;
}) {
  const [expanded, setExpanded] = useState(true);
  const { threadEdits, unreadEnabled } = useStore();
  const navigation = useNavigation();
  const displayed = thread && (threadEdits?.project(thread) ?? thread);
  const title = displayed?.title ?? start?.title;
  const standalone = !(onOpen || embedded);
  // The standalone screen names the thread in its navigation bar, following renames.
  useEffect(() => { if (standalone && title) navigation.setOptions({ title }); }, [navigation, standalone, title]);
  if (thread || start) {
    const pending = start?.accepted ? 'Loading history…' : 'draft';
    // A newly started inline thread can inherit an existing thread's title and
    // count. Name its placement, visibly and for VoiceOver, not by a shared title.
    const startedHere = !!start?.accepted && !!displayed;
    const trailing = [startedHere ? 'Started here' : undefined, displayed?.state === 'closed' ? 'Closed' : undefined].filter(Boolean).join(' · ') || undefined;
    const summary = standalone ? <T dim>{displayed ? threadSummaryLine(displayed, ' · ', displayed.state !== 'closed', unreadEnabled) : pending}</T>
      : <ThreadRow bold title={title!} label={`${title}, ${displayed ? threadSummaryLine(displayed, ', ', true, unreadEnabled) : pending}${startedHere ? ', started here' : ''}`} unread={displayed?.unread} reminder={!!unreadEnabled && !!reminderFromSummary(displayed)}
        trailing={displayed ? trailing : pending} expanded={expanded} toggle={() => setExpanded(value => !value)} />;
    // A framed (docked) thread puts its summary line at the top of the scrolling history.
    if (frame) return <ThreadBody client={client} thread={thread} start={start} placement={placement} onOpen={onOpen} frame={(content, dock) => frame(<>{summary}{content}</>, dock)} />;
    return <>
      {summary}
      {expanded || standalone ? <ThreadBody client={client} thread={thread} start={start} placement={placement} onOpen={onOpen} /> : null}
    </>;
  }
  const status = issue ? <>
    <ThreadReadIssue message={issue} />
    <Button label="Refresh thread" onPress={reload} disabled={loading} />
  </> : <T dim>Loading thread…</T>;
  return frame ? frame(status) : status;
}

function threadSummaryLine(thread: ThreadSummary, separator = ' · ', includeState = true, includeReminder = true): string {
  const count = thread.message_count;
  return [
    count === undefined ? undefined : `${count} message${count === 1 ? '' : 's'}`,
    includeState ? thread.state : undefined,
    thread.unread ? 'unread' : undefined,
    includeReminder && reminderFromSummary(thread) ? 'marked unread reminder' : undefined,
  ].filter(Boolean).join(separator);
}

/** One thread row: an unread dot, the title, and Closed. Counts and state stay in the spoken label. */
function ThreadRow({ title, label, unread, reminder, trailing, expanded, toggle, bold }: {
  title: string; label: string; unread?: boolean; reminder?: boolean; trailing?: string; expanded: boolean; toggle: () => void; bold?: boolean;
}) {
  const appearance = useTextAppearance();
  const titleLines = useTitleLines();
  return <Pressable accessibilityRole="button" accessibilityState={{ expanded }} accessibilityLabel={label} onPress={toggle}
    accessibilityValue={{ text: expanded ? 'Expanded' : 'Collapsed' }} accessibilityHint={expanded ? `Hides ${title}` : `Shows ${title}`}
    style={{ minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 8 }}>
    <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: unread ? appearance.color(theme.accent) : 'transparent' }} />
    <T bold={bold} color={theme.accent} numberOfLines={titleLines} style={{ flex: 1 }}>{title}</T>
    {reminder ? <T dim>Marked unread</T> : null}
    {trailing ? <T dim numberOfLines={1}>{trailing}</T> : null}
  </Pressable>;
}

/** Each source episode starts its own independent threads. A listed row Starts only on its exact
 * source episode, with the recipient its StartContext names: no Inbox or roster agent selects it,
 * so system and typed notices without an assigned agent Start too. A pending row names one
 * handed-off attempt by its delivery identity (threadStartAttemptScope: source, episode and new
 * thread ID): its live receipt with the exact agent it signed, or after a restart its read-only
 * saved draft. A pending row never authorizes a new Start or rereads its recipient.
 */
export function InboxThread({ sourceId, origin, initialTitle, onOpen, pendingScope }: {
  sourceId: string; origin?: Pick<ThreadStartOrigin, 'sourceId' | 'episode'>;
  initialTitle: string; onOpen: OpenThread; pendingScope?: string;
}) {
  const { threadClient, threadActions, threadDeliveries, draftCache } = useStore();
  if (!threadClient) return null;
  const retained = pendingScope ? threadDeliveries?.get(pendingScope) : undefined;
  const restored = pendingScope && !retained && draftCache
    ? savedThreadStarts(draftCache.current).find(start => threadStartAttemptScope(start.origin, start.target) === pendingScope) : undefined;
  if (restored) return <RestoredThreadStart start={restored} />;
  const shownOrigin = pendingScope ? retained?.start?.origin : origin;
  if (!shownOrigin) return <T dim>Thread context is unavailable for this Inbox item.</T>;
  return <InboxThreadStart key={pendingScope ?? threadStartScope(shownOrigin)} client={threadClient}
    origin={shownOrigin} pending={pendingScope && retained?.start ? { target: retained.target, agentId: retained.start.origin.agentId } : undefined}
    initialTitle={pendingScope ? retained?.start?.title ?? initialTitle : initialTitle} onOpen={onOpen}
    showHistory={!pendingScope} canStart={!pendingScope && !!origin && threadActions.has('thread.start')} />;
}

/** A retained request-history row's Start: the row's exact source episode and its StartContext
 * recipient. No current Inbox item or agent is required; the row lists no thread collection.
 */
function HistoryStart({ episode, title }: { episode: RequestHistoryItem['key']; title: string }) {
  const { threadClient, threadActions } = useStore();
  const navigation = useNavigation<NativeStackNavigationProp<StackParams>>();
  if (!threadClient || !threadActions.has('thread.start')) return null;
  const origin = { sourceId: episode.source_id, episode: episode.episode };
  return <InboxThreadStart key={threadStartScope(origin)} client={threadClient} origin={origin} initialTitle={title}
    onOpen={(id, title) => navigation.navigate('Thread', { id, title })} showHistory={false} canStart />;
}

function RestoredThreadStart({ start }: { start: SavedThreadStart }) {
  const { draftCache, saveDrafts } = useStore();
  const [discarded, setDiscarded] = useState(false);
  if (discarded) return null;
  const key = threadDraftKey(threadStartSlot(start.target));
  const draft = readThreadDraft(draftCache.current.get(key));
  return <View style={{ gap: 6 }}>
    <T bold>{start.title}</T>
    <T dim>Delivery status unknown after restart. This saved draft cannot retry the original send. Copy it into a new Start to send a separate message; the original may already have arrived.</T>
    {draft.quote ? <T dim>Quoted · {draft.quote.from} · {draft.quote.text}</T> : null}
    <T selectable>{draft.text}</T>
    <Button label="Copy message" onPress={() => { void Clipboard.setStringAsync(draft.text); }} />
    <Button label="Discard saved draft" onPress={() => {
      draftCache.current.delete(key);
      draftCache.current.delete(startRecoveryKey(start.target));
      saveDrafts();
      setDiscarded(true);
    }} />
  </View>;
}

/** Every handed-off Start attempt, each under its exact source, episode and new thread ID: live
 * receipts first, then attempts saved before a restart. Attempts of a listed episode are recovered
 * here too, beside that row's fresh Start; none folds into another episode or blocks a new Start.
 */
export function PendingInboxThreads({ onOpen }: { onOpen: OpenThread }) {
  const { threadClient, threadDeliveries, draftCache } = useStore();
  if (!threadClient) return null;
  const pending = new Map([...(threadDeliveries?.entries() ?? [])].flatMap(([scope, delivery]) =>
    delivery.start ? [[delivery.target, { scope, target: delivery.target, ...delivery.start }] as const] : []));
  if (draftCache) for (const start of savedThreadStarts(draftCache.current)) {
    if (!pending.has(start.target)) pending.set(start.target, { ...start, scope: threadStartAttemptScope(start.origin, start.target) });
  }
  if (!pending.size) return null;
  return <View style={{ padding: 12, gap: 6 }}>
    <T bold>Unconfirmed thread starts</T>
    {[...pending.values()].map(start => <View key={start.scope} style={{ gap: 4 }}>
      <T dim>Original recipient · {start.origin.agentId}</T>
      <InboxThread sourceId={start.origin.sourceId} pendingScope={start.scope} initialTitle={start.title} onOpen={onOpen} />
    </View>)}
  </View>;
}

type ThreadStartDraft = {
  target?: string; cacheKey: string; title: string; accepted: boolean;
  /** Exact source, episode and the agent this attempt signs: a pending attempt's original agent, or
   * the recipient its StartContext names. Undefined while that context is unread; the composer is
   * disabled until it is. */
  origin?: ThreadStartOrigin;
  onAccepted: (id: string) => void; onCancel: () => void;
  /** Called only after an unconfirmed dispatch, with the new thread ID and the rendered draft it sent. */
  onSubmitted?: (id: string, serialized: string) => void;
  /** Called after a fresh Start was definitively refused for a stale recipient. It rereads the
   * StartContext to show the new recipient; it never sends. Absent for a handed-off attempt. */
  onRecipientRefused?: () => void;
};

function InboxThreadStart({ client, origin, pending, initialTitle, onOpen, canStart, showHistory = true }: {
  client: ThreadClient; origin: Pick<ThreadStartOrigin, 'sourceId' | 'episode'>;
  /** A handed-off attempt: its new thread ID and the exact agent it signed. Its recipient is never reread. */
  pending?: { target: string; agentId: string };
  initialTitle: string; onOpen: OpenThread; canStart: boolean; showHistory?: boolean;
}) {
  const { status, threadRefresh, threadDeliveries, draftCache, draftsReady, saveDrafts } = useStore();
  const pendingTarget = pending?.target;
  const scope = threadStartScope(origin);
  const cacheKey = threadDraftKey(pendingTarget ? threadStartSlot(pendingTarget) : scope);
  const [local, setLocal] = useState<{ client: ThreadClient; target?: string; agentId?: string; accepted: boolean; generation: number; refused?: boolean }>();
  const current = local?.client === client ? local : undefined;
  const target = current?.target ?? pendingTarget;
  const accepted = current?.accepted === true;
  const generation = current?.generation ?? 0;
  // One placement per handed-off attempt; a fresh origin composer advances with each Start.
  const placement = pendingTarget ? threadStartSlot(pendingTarget) : `${scope}:${generation}`;
  const delivery = target ? threadDeliveries?.get(threadStartAttemptScope(origin, target)) : undefined;
  const show = !!current || !!delivery || draftsReady && draftCache.current.has(cacheKey);
  // A fresh, unsent Start reads who it reaches before composing; every new Start reads again.
  const reading = !pending && show && !accepted;
  const reader = useMemo(() => new ThreadStartContextReader(client, { source_id: origin.sourceId, episode: origin.episode }),
    [client, origin.sourceId, origin.episode]);
  const context = useSyncExternalStore(reader.subscribe, reader.getSnapshot);
  useFocusEffect(useCallback(() => {
    if (!reading || status !== 'online') return;
    void reader.load();
    return () => reader.close();
  }, [reader, reading, generation, status]));
  // Only the StartContext selects a fresh Start's recipient; a handed-off or accepted attempt keeps
  // the exact agent it signed.
  const agentId = pending ? pending.agentId : accepted ? current?.agentId
    : reading && context.kind === 'ready' ? context.context.start_recipient.agent_id : undefined;
  const signed = agentId ? { ...origin, agentId } : undefined;
  const draft: ThreadStartDraft | undefined = show ? {
    target, cacheKey, origin: signed, accepted, title: threadTitleFromAsk(initialTitle),
    onAccepted: id => {
      if (pendingTarget) {
        draftCache.current.delete(cacheKey);
        draftCache.current.delete(startRecoveryKey(pendingTarget));
        saveDrafts();
      }
      setLocal({ client, target: id, agentId: signed?.agentId, accepted: true, generation });
    },
    onSubmitted: (id, serialized) => {
      // Use the dispatch snapshot, never later typing from a remounted composer.
      if (!pendingTarget) {
        draftCache.current.set(threadDraftKey(threadStartSlot(id)), serialized);
        draftCache.current.set(startRecoveryKey(id), JSON.stringify({ target: id, origin: signed, title: threadTitleFromAsk(initialTitle) }));
        if (draftCache.current.get(cacheKey) === serialized) draftCache.current.delete(cacheKey);
      }
      saveDrafts();
      setLocal(undefined);
    },
    onCancel: () => {
      draftCache.current.delete(cacheKey);
      saveDrafts();
      setLocal(undefined);
    },
    onRecipientRefused: pending ? undefined : () => {
      setLocal({ client, accepted: false, generation, refused: true });
      void reader.load(true);
    },
  } : undefined;
  const recipient = !reading ? null
    : context.kind === 'loading' ? <T dim>Reading who this Start reaches…</T>
    : context.kind === 'blocked' ? <ThreadReadIssue message={context.message} />
    : context.kind === 'error' ? <View style={{ gap: 6 }}>
      <ThreadReadIssue message={context.message} />
      <Button label="Retry" accessibilityLabel="Retry reading the Start recipient" disabled={status !== 'online'}
        onPress={() => void reader.load(current?.refused === true)} />
    </View>
    : <StartRecipientPreview recipient={context.context.start_recipient} changed={context.changed} />;
  return <View style={{ gap: 6 }}>
    {showHistory ? <ThreadGroup context={{ origin_source: origin.sourceId, origin_episode: origin.episode }} openThread={onOpen} excludeId={accepted ? target : undefined} /> : null}
    {recipient}
    {draft ? <InlineThread key={placement} client={client} id={target} placement={`inbox:${placement}`} refresh={threadRefresh}
      onOpen={target && accepted ? () => onOpen(target, draft.title) : undefined} start={draft} embedded /> : null}
    {canStart && (!draft || accepted || !!delivery) ? <Button label={accepted || delivery ? 'Start another thread' : 'Start thread'} disabled={status !== 'online'}
      onPress={() => setLocal(previous => ({ client, accepted: false, generation: (previous?.generation ?? 0) + 1 }))} /> : null}
  </View>;
}

function InlineThread({ client, id, placement, refresh, onOpen, start, embedded, frame }: {
  client: ThreadClient; id?: string; placement: string; refresh: number; onOpen?: () => void; start?: ThreadStartDraft; embedded?: boolean; frame?: ThreadFrame;
}) {
  const page = useThreadSummary(client, id, refresh, !start || start.accepted);
  // Navigation or Inbox mounting is the visit's entry, even while the summary loads.
  useUnreadVisitEntry({ scope: client, placementKey: placement, threadId: id, actor: useStore().actor, summary: page.items[0] });
  return <ThreadDetailContent thread={page.items[0]} issue={page.issue} loading={page.loading} reload={page.reload} client={client} placement={placement} onOpen={onOpen} start={start} embedded={embedded} frame={frame} />;
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

/** Only whole rendered lines are evidence, never paragraph spacing, rules or margins.
 * A body offers its bounds only while its last line and two of its lines are fully inside
 * the viewport; a one-line body needs its only line. Otherwise it fails closed with no bounds.
 */
function renderedLineEvidence(viewport: ThreadBounds | undefined, bounds: ThreadBounds | undefined, lines: readonly MarkdownLine[] | undefined): { bounds?: ThreadBounds; lineHeight?: number } {
  if (!viewport || !bounds || !lines?.length) return {};
  const inside = (line: MarkdownLine) => bounds.y + line.y >= viewport.y && bounds.y + line.y + line.height <= viewport.y + viewport.height;
  const visible = lines.filter(inside).length;
  const lineHeight = Math.max(...lines.map(line => line.height));
  return visible >= Math.min(lines.length, 2) && inside(lines[lines.length - 1]) ? { bounds, lineHeight } : { lineHeight };
}

function threadTextIssue(content: string): string | undefined {
  if (content.startsWith('doc/')) return 'Use message text here.';
  if (utf8Bytes(content) > INLINE_MAX_BYTES) return `Keep the message under ${INLINE_MAX_BYTES.toLocaleString()} UTF-8 bytes.`;
}

/** Mounted error text is announced on iOS too; Android's live region alone is insufficient. */
function ThreadReadIssue({ message }: { message: string }) {
  const target = useRef<View>(null);
  useEffect(() => {
    AccessibilityInfo.announceForAccessibility(message);
    if (message.startsWith('You no longer have access to this thread.')) {
      const handle = target.current && findNodeHandle(target.current);
      if (handle != null) AccessibilityInfo.setAccessibilityFocus(handle);
    }
  }, [message]);
  return <View ref={target} accessible accessibilityRole="alert" accessibilityLabel={message} accessibilityLiveRegion="polite">
    <T color={theme.waiting}>{message}</T>
  </View>;
}

function ThreadActionFeedback({ result, busy, onRetry }: { result?: ThreadEditResult; busy: boolean; onRetry: () => void }) {
  useEffect(() => {
    if (result?.how === 'failed') AccessibilityInfo.announceForAccessibility(
      `${result.action ? `Couldn't ${result.action} this thread. ` : ''}${result.error}`,
    );
  }, [result]);
  if (result?.how !== 'failed' && result?.how !== 'unconfirmed') return null;
  return ConversationFeedback({
    color: theme.waiting,
    heading: result.how === 'failed' && result.action ? `Couldn't ${result.action} this thread.` : undefined,
    message: result.how === 'failed' ? result.error : 'The original connection has not confirmed this change.',
    recovery: result.how === 'unconfirmed' ? { label: 'Retry original change', disabled: busy, onPress: onRetry } : undefined,
  });
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

// The same thread view renders inside a conversation, on its own screen and around a
// marked thread's return message. `placement` names where this body is shown, stably
// across rerenders and remounts; `around` seeds the history with a current anchor page.
function ThreadBody({ client, thread, start, placement, around, onOpen, revealComposer, frame }: {
  client: ThreadClient; thread?: ThreadSummary; start?: ThreadStartDraft; placement: string; around?: ThreadTimelinePage; onOpen?: () => void; revealComposer?: (row: View) => void; frame?: ThreadFrame;
}) {
  const { actions, threadActions, status, threadRefresh, threadDeliveries, threadEdits, actor, data, feed, unreadClient } = useStore();
  const displayed = thread && (threadEdits?.project(thread) ?? thread);
  const flash = thread && threadEdits?.flash(thread.id);
  const id = thread?.id ?? start?.target;
  // An anchored page is its own first page; older pages continue from its cursor. The
  // live head window is not followed there, so it can never replace the anchored page.
  const read = useCallback(async (cursor: string | undefined, signal: AbortSignal) => thread
    ? around && cursor === undefined ? { value: around } : client.threadTimeline(thread.id, { cursor, limit: THREAD_WINDOW_SIZE }, { signal })
    : { value: { items: [], next_cursor: null } }, [around, client, id, !!thread]);
  const follow = useCallback((handlers: ThreadWindowHandlers<ThreadTimelineEvent>) =>
    thread && !around && feed?.belongsTo(client) ? feed.followThreadTimeline(thread.id, handlers) : undefined, [around, client, feed, id, !!thread]);
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
  const now = useSyncExternalStore(threadClock.subscribe, threadClock.getSnapshot);
  const timeline = [...page.items].reverse();
  const chooseQuote = (message: ThreadMessage) => {
    setQuote(message);
    setQuoteFocus(previous => previous + 1);
  };
  const { editable, feedback: editFeedback, busy: editBusy, run: runEdit, retry: retryEdit, setResult: setEditResult } = useThreadEditing(thread);
  const [renameTitle, setRenameTitle] = useState<string>();
  const canMarkRead = !!thread?.unread && !!actor && threadActions.has('thread.read');
  // Live grants from the current capability set, re-checked again when an edit is sent.
  const unreadAPI = thread ? unreadClient.forThread(thread) : pendingUnreadAPI;
  const canClearReminder = unreadAPI.state === 'ready' && !!actor && threadActions.has('thread.clear-unread');
  const canMarkUnread = canClearReminder && threadActions.has('thread.mark-unread');
  const reminder = unreadAPI.state === 'ready' ? reminderFromSummary(thread) : undefined;
  const clearReminder = canClearReminder ? reminderClear(unreadAPI, thread) : undefined;
  const readCandidates = canMarkRead ? page.items.filter((event): event is ThreadMessage =>
    event.kind === 'message' && !!event.content.trim() && !ownMessage(event.actor_id)) : [];
  // Rendered body lines, as laid out; a body without them is never visible evidence.
  const bodyLines = useRef(new Map<string, readonly MarkdownLine[]>());
  // One ref callback per message for as long as it is listed. React detaches and re-attaches a
  // callback whose identity changes, so an inline callback would discard the laid-out lines on
  // every rerender (the shared clock ticks each second); Markdown reports lines only on layout.
  const messageRefs = useRef(new Map<string, (view: View | null) => void>());
  for (const messageId of messageRefs.current.keys()) if (!messages.has(messageId)) messageRefs.current.delete(messageId);
  const messageRef = (messageId: string) => {
    let attach = messageRefs.current.get(messageId);
    if (!attach) {
      attach = view => {
        if (view) { messageViews.current.set(messageId, view); return; }
        messageViews.current.delete(messageId);
        bodyLines.current.delete(messageId);
      };
      messageRefs.current.set(messageId, attach);
    }
    return attach;
  };
  const visit = useUnreadVisit({
    scope: client, placementKey: placement, threadId: id, actor, summary: thread, api: unreadAPI,
    canRead: canMarkRead && status === 'online', canEdit: canClearReminder && status === 'online',
    feedback: setEditResult,
    read: ids => thread ? actions.markThreadRead(thread, ids) : Promise.resolve({ how: 'failed', error: 'This thread has not been sent yet.' }),
    measure: async () => {
      const top = Math.max(headerHeight, insets.top);
      const candidates = page.items.filter((event): event is ThreadMessage => event.kind === 'message' && !!event.content.trim());
      const [viewport, measured] = await Promise.all([
        measureThreadViewport(viewports, { x: insets.left, y: top, width: width - insets.left - insets.right, height: height - top - Math.max(tabBarHeight, insets.bottom) }),
        Promise.all(candidates.map(async ({ id }) => ({ id, bounds: await measureThreadBounds(messageViews.current.get(id)), lines: bodyLines.current.get(id) }))),
      ]);
      const rows = measured.map(({ id, bounds, lines }) => ({ id, ...renderedLineEvidence(viewport, bounds, lines) }));
      const messageIds = viewport ? visibleThreadMessages(viewport, rows, THREAD_WINDOW_SIZE) : [];
      return { messageIds, readMessageIds: messageIds.filter(id => readCandidates.some(message => message.id === id)) };
    },
  });
  const linesLaidOut = (messageId: string, lines: readonly MarkdownLine[]) => {
    bodyLines.current.set(messageId, lines);
    visit.refresh();
  };
  const markVisibleRead = () => void runEdit(() => visit.measure('read'));
  const markUnread = () => void runEdit(() => visit.measure('unread'));
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
  const rows = timeline.map(event => {
      const name = actorLabel(event.actor_id);
      const age = threadRelativeTime(event.accepted_at_unix_ms, now);
      const exactTime = threadExactTime(event.accepted_at_unix_ms);
      if (event.kind !== 'message') return <View key={event.id} testID={`thread-event:${event.id}`} accessible accessibilityRole="text" accessibilityLabel={`${name}, ${exactTime}. ${eventLine(event)}`}>
        <T dim><T bold>{name}</T><Text accessibilityElementsHidden importantForAccessibility="no"> · {age}</Text> · {eventLine(event)}</T>
      </View>;
      const quoted = event.thread_quote_id ? messages.get(event.thread_quote_id) : undefined;
      const quoteLabel = event.thread_quote_id ? quoted ? `Reply to ${actorLabel(quoted.actor_id)} · ${quoted.content}. ` : 'Reply to an earlier message. ' : '';
      const reply = canReply ? () => chooseQuote(event) : undefined;
      const readHere = canMarkRead && !ownMessage(event.actor_id) && editable;
      const returnHere = reminder?.anchor_message_id === event.id;
      const row = <Pressable key={event.id} testID={`thread-message:${event.id}`} onLongPress={reply} accessible accessibilityLabel={`${returnHere ? 'Your marked unread return message. ' : ''}${name}, ${exactTime}. ${quoteLabel}${event.content}`}
        accessibilityActions={[
          ...(reply ? [{ name: 'reply', label: 'Reply to this message' }] : []),
          ...(readHere ? [{ name: 'markRead', label: 'Mark visible replies read' }] : []),
        ]}
        onAccessibilityAction={action => {
          if (action.nativeEvent.actionName === 'reply') reply?.();
          if (action.nativeEvent.actionName === 'markRead' && readHere) markVisibleRead();
        }}>
        {ConversationMessage({
          before: returnHere ? <T color={theme.accent}>Marked unread · return here</T> : undefined,
          heading: <>{name}<Text accessibilityElementsHidden importantForAccessibility="no"> · {age}</Text></>, headingBold: true,
          beforeBody: event.thread_quote_id ? <T dim numberOfLines={1}>{quoted ? `Reply to ${actorLabel(quoted.actor_id)} · ${quoted.content}` : 'Reply to an earlier message'}</T> : undefined,
          text: event.content, selectable: !reply, bodyRef: messageRef(event.id), onBodyLayout: visit.refresh,
          onLines: lines => linesLaidOut(event.id, lines),
        })}
      </Pressable>;
      return <ContextMenu key={event.id} title={exactTime} actions={readHere ? [
        ...(reply ? [{ id: 'reply', title: 'Reply to this message', run: reply }] : []),
        { id: 'markRead', title: 'Mark visible replies read', run: markVisibleRead },
      ] : []}>{row}</ContextMenu>;
  });
  const history = page.issue ? <><ThreadReadIssue message={page.issue} /><Button label="Retry" accessibilityLabel={`Retry loading ${displayed?.title ?? start?.title ?? 'thread'}`} onPress={page.reload} disabled={page.loading} /></> : !start && !page.items.length ? <T dim>{page.loading ? 'Loading history…' : 'No thread messages yet.'}</T> : null;
  const newer = around?.previous_cursor && onOpen ? <T dim>Newer messages follow. Open the thread for the latest.</T> : null;
  const markRead = <View style={{ gap: 6 }}>
    {readCandidates.length || reminder && canClearReminder ? <Button label="Mark read" disabled={!editable} onPress={markVisibleRead} /> : null}
    {clearReminder ? <Button label="Clear reminder" hint="Removes the marked unread reminder without reading replies" disabled={!editable} onPress={() => void runEdit(clearReminder)} /> : null}
    {thread && canMarkUnread ? <Button label="Mark unread" disabled={!editable} onPress={markUnread} /> : null}
    {reminder ? <T color={theme.accent}>Marked unread · return message kept</T> : null}
  </View>;
  const closedHint = closed ? <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
    <View accessible accessibilityRole="text" accessibilityLabel={canReply ? 'Closed, replying keeps it closed' : 'Closed'} style={{ flex: 1 }}>
      <T dim>{canReply ? 'Closed · replying keeps it closed' : 'Closed'}</T>
    </View>
    {threadActions.has('thread.reopen') ? <Button label="Reopen thread" hint={`Reopens ${displayed?.title}`} disabled={!editable} onPress={changeState} /> : null}
  </View> : null;
  const composer = canReply || id && threadDeliveries?.has(id) || start ? <ThreadComposer key={start?.cacheKey ?? id} thread={thread} start={start} canReply={canReply} contextualTitle={onOpen ? displayed?.title ?? start?.title : undefined}
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
  const editStatus = <ThreadActionFeedback result={editFeedback} busy={editBusy} onRetry={retryEdit} />;
  const threadButtons = thread ? <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
    {threadActions.has('thread.rename') ? <Button label="Rename" hint={`Renames ${displayed!.title}`} onPress={() => setRenameTitle(displayed!.title)} disabled={!editable} /> : null}
    {!closed && threadActions.has('thread.close') ? <Button label="Close thread" hint={`Closes ${displayed!.title}`} disabled={!editable} onPress={changeState} /> : null}
    {onOpen ? <Button label="Open thread" hint={`Opens ${displayed!.title}`} onPress={onOpen} /> : null}
    {!page.live ? <Button label="Refresh history" onPress={page.reload} disabled={page.loading || status !== 'online'} /> : null}
  </View> : null;

  // The thread screen docks the confirmation, closed hint and the one composer below its scrolling
  // history; embedded threads keep the same elements inline, in reading order. Request history
  // follows the thread's own controls, outside every measured message row.
  const requestHistory = thread ? <RequestHistory client={client} threadId={thread.id} StartAction={HistoryStart} /> : null;
  if (frame) return frame(<View style={{ gap: 6 }}>{earlier}{rows}{newer}{history}{markRead}{renameField}{editStatus}{threadButtons}{requestHistory}</View>,
    flashLine || closedHint || composer || unavailable ? <View style={{ gap: 6 }}>{flashLine}{closedHint}{composer}{unavailable}</View> : undefined);
  return <View style={{ gap: 6, paddingBottom: 8 }}>
    {earlier}{rows}{newer}{history}{markRead}{closedHint}{composer}{unavailable}{flashLine}{renameField}{editStatus}{threadButtons}{requestHistory}
  </View>;
}

/** Clear reminder for a return message with no body here: a pending or unavailable anchor.
 * Read-only connections get no control, but a retained clear still offers its retry.
 */
function ClearReminder({ thread }: { thread: ThreadSummary }) {
  const { actor, threadActions, unreadClient } = useStore();
  const { editable, feedback, busy, run, retry } = useThreadEditing(thread);
  const api = unreadClient.forThread(thread);
  const clear = api.state === 'ready' && !!actor && threadActions.has('thread.clear-unread') ? reminderClear(api, thread) : undefined;
  if (!clear && !feedback) return null;
  return <View style={{ gap: 6 }}>
    {clear ? <Button label="Clear reminder" hint="Removes the marked unread reminder without reading replies" disabled={!editable} onPress={() => void run(clear)} /> : null}
    <ThreadActionFeedback result={feedback} busy={busy} onRetry={retry} />
  </View>;
}

/** An explicit clear names exactly the mark IDs the current summary shows, never IDs from a
 * receipt, a seek or the visit's entry capture, and sends them through the store's bound
 * delivery, which re-checks the live grant. It never seeks, measures, waits out a dwell or
 * acknowledges messages, so it works whether the return message is current, scrolled away,
 * pending or unavailable. The next summary, not the receipt, removes the reminder.
 */
function reminderClear(api: UnreadAPI, thread: ThreadSummary | undefined): (() => Promise<ThreadEditResult>) | undefined {
  const actions = api.state === 'ready' ? api.actions : undefined;
  const reminder = actions && reminderFromSummary(thread);
  if (!actions || !reminder) return undefined;
  const observed_marks = reminder.marks.map(mark => mark.mark_id);
  return () => actions.edit({ kind: 'clear', observed_marks });
}

/** One thread's edit state, shared by its body and by a reminder notice without one. The store
 * retains a bound unread mark or clear under `unread/<thread>`, as it retains metadata edits
 * under threadEditScope, so a remount recovers either original request. Explicit precedence:
 * the unread delivery's retry is offered first; the metadata edit's retry follows once that is
 * confirmed. Any retained delivery blocks new edits until it is resolved.
 */
function useThreadEditing(thread: ThreadSummary | undefined) {
  const { status, threadDeliveries } = useStore();
  const [editing, setEditing] = useState(false);
  const [result, setResult] = useState<ThreadEditResult>();
  const editDelivery = thread ? threadDeliveries?.get(threadEditScope(thread.id)) : undefined;
  const unreadDelivery = thread ? threadDeliveries?.get(`unread/${thread.id}`) : undefined;
  const held = [unreadDelivery, editDelivery].filter(delivery => delivery !== undefined);
  const recovered = held.find(delivery => delivery.retry)?.retry;
  const feedback = recovered ? { how: 'unconfirmed' as const, retry: recovered } : result;
  const editable = status === 'online' && !editing && !held.length && feedback?.how !== 'unconfirmed';
  const run = async (send: () => Promise<ThreadEditResult>, retained = false) => {
    setEditing(true);
    setResult(undefined);
    try { const next = await send(); setResult(retained && next.how === 'unconfirmed' ? undefined : next); }
    finally { setEditing(false); }
  };
  const busy = editing || held.some(delivery => !delivery.retry) || status !== 'online';
  const retry = () => { if (feedback?.how === 'unconfirmed') void run(feedback.retry, !!recovered); };
  return { editable, feedback, busy, run, retry, setResult };
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

function ThreadComposer({ thread, start, canReply, contextualTitle, selection, selectionVersion, revealComposer }: {
  thread?: ThreadSummary; start?: ThreadStartDraft; canReply: boolean; contextualTitle?: string; selection?: Quote; selectionVersion: number; revealComposer?: (row: View) => void;
}) {
  const { actions, draftCache, draftsReady, draftsOwner, draftGeneration, saveDrafts, status, threadDeliveries } = useStore();
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
  const sentTarget = useRef<string | undefined>(undefined);
  const [failure, setFailure] = useState<ThreadEditResult>();
  // A Start's receipt belongs to its attempt (source, episode and new thread ID), never to the
  // shared origin: a first composer has none, so a handed-off attempt never blocks the next Start.
  const deliveryTarget = id ?? sentTarget.current;
  const delivery = threadDeliveries?.get(!thread && start ? deliveryTarget && start.origin ? threadStartAttemptScope(start.origin, deliveryTarget) : '' : deliveryTarget!);
  const sending = !!delivery && !delivery.retry;
  const feedback = delivery?.retry ? { how: 'unconfirmed' as const, retry: delivery.retry } : failure;
  // A fresh Start composes only once its StartContext names the recipient it signs.
  const enabled = canReply && status === 'online' && !sending && !delivery && (!!thread || !start || !!start.origin);
  // A handed-off attempt keeps its signed agent and thread ID. Once its exact retry is refused for a
  // stale recipient, it never sends again: its text moves to a separate, freshly read Start.
  const staleAttempt = !thread && !!start?.target && failure?.how === 'failed' && !!failure.refreshStartContext;
  const content = draft.text.trim();
  const issue = threadTextIssue(content);
  useEffect(() => { if (canReply && issue) AccessibilityInfo.announceForAccessibility(issue); }, [canReply, issue]);

  function storeDraft(next: ThreadDraft) {
    touched.current = true;
    setDraft(next);
    if (next.text || next.quote) draftCache.current.set(key, JSON.stringify(next));
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
    AccessibilityInfo.announceForAccessibility(`Replying to ${selection.from}`);
    // A selection event, rather than a new render, replaces the draft's quote.
  }, [selectionVersion]);

  const send = async (run: () => Promise<ThreadEditResult>) => {
    const submittedDraft = start && !thread ? JSON.stringify(draft) : undefined;
    const submittedOwner = draftsOwner.current;
    const submittedGeneration = draftGeneration.current;
    const next = await run();
    // The shared map is reset in place on pairing/actor changes. An old send
    // must neither persist its text for the next owner nor clear their typing.
    if (start && !thread && (draftsOwner.current !== submittedOwner || draftGeneration.current !== submittedGeneration)) return;
    if (next.how === 'unconfirmed' && start && !thread) start.onSubmitted?.(sentTarget.current!, submittedDraft!);
    if (next.how === 'sent') {
      if (!start || thread || draftCache.current.get(key) === submittedDraft) storeDraft({ text: '' });
      if (start && !thread) {
        start.onAccepted(sentTarget.current!);
      }
    }
    setFailure(next.how === 'failed' ? next : undefined);
    // Only after the owner/generation fences: reread the recipient, show it, and wait for a new
    // explicit send. A handed-off attempt has no callback and is never retargeted.
    if (next.how === 'failed' && next.refreshStartContext && start && !thread) start.onRecipientRefused?.();
  };

  return <View style={{ gap: 6 }}>
    {canReply ? <ThreadComposeField contextualTitle={contextualTitle} inputRef={input} rowRef={revealComposer ? row : undefined} onReveal={revealComposer ? reveal : undefined} text={draft.text} onChange={text => storeDraft({ ...draft, text })}
      before={draft.quote ? <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6, marginBottom: 6 }}>
      <T dim numberOfLines={1} style={{ flex: 1 }}>Replying to {draft.quote.from} · {draft.quote.text}</T>
      <Button label="Remove quote" disabled={!enabled} onPress={() => storeDraft({ text: draft.text })} />
    </View> : null}
      after={issue ? <T color={theme.waiting} style={{ marginTop: 6 }}>{issue}</T> : null}
      onSend={() => {
        if (!enabled || !content || issue || !thread && (!start?.origin || start.accepted)) return;
        void send(() => {
          if (thread) return actions.replyToThread(thread, content, draft.quote?.id);
          const target = start!.target ?? `thread/${Crypto.randomUUID()}`;
          sentTarget.current = target;
          return actions.startThread(target, start!.origin!, start!.title, content);
        });
      }} enabled={enabled} canSend={(!!thread || !start?.accepted) && !staleAttempt && !!content && !issue}
      autoFocus={!!start && !thread} onCancel={start && !thread && !start.accepted && !delivery && !sending ? start.onCancel : undefined} /> : null}
    {staleAttempt ? <View style={{ gap: 6 }}>
      <T dim>This send's recipient changed, so it cannot be retried. Copy the message into a new Start to send it separately.</T>
      <Button label="Copy message" onPress={() => { void Clipboard.setStringAsync(draft.text); }} />
    </View> : null}
    <ThreadActionFeedback result={feedback} busy={sending || status !== 'online'} onRetry={() => {
      if (delivery?.retry) { sentTarget.current = delivery.target; void send(delivery.retry); }
    }} />
  </View>;
}

function ThreadComposeField({ contextualTitle, inputRef, rowRef, onReveal, text, onChange, onSend, onCancel, enabled, canSend, autoFocus, before, after }: {
  contextualTitle?: string; inputRef?: Ref<TextInput>; rowRef?: Ref<View>; onReveal?: () => void; text: string; onChange: (text: string) => void; onSend: () => void;
  onCancel?: () => void; enabled: boolean; canSend: boolean; autoFocus?: boolean; before?: ReactNode; after?: ReactNode;
}) {
  return ConversationComposer({
    text, onChange, onSend, inputRef, rowRef, onReveal, enabled, canSend, autoFocus, onCancel, before, after,
    label: contextualTitle ? `Message ${contextualTitle}` : 'Message thread',
    sendAccessibilityLabel: contextualTitle ? `Send reply to ${contextualTitle}` : undefined,
  });
}
