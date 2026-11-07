import { useCallback, useRef, useState } from 'react';
import { Alert, Pressable, ScrollView, useWindowDimensions, View } from 'react-native';
import { useFocusEffect } from '@react-navigation/native';
import { ago } from '@fractal/views';
import { ClientError, type ThreadSummary, type ThreadTimelineEvent } from '../../../clients/typescript/fractal-client';
import type { RootScreen } from '../navigation';
import { errorText, useStore, type ThreadEditResult } from '../store';
import { theme } from '../theme';
import { Button, SchemeScreen, T, useTextAppearance } from '../ui';

type ThreadPage<T> = { items: T[]; next_cursor: string | null };
type ReadPage<T> = (cursor: string | undefined, signal: AbortSignal) => Promise<{ value: ThreadPage<T> }>;

// Follow the focused screen, reset old cursors after a mutation, and discard responses
// from the previous member or context. No client-owned copy survives those boundaries.
function useThreadPage<T>(read: ReadPage<T>, refresh: number) {
  const active = useRef<AbortController | null>(null);
  const [result, setResult] = useState<{ read: ReadPage<T>; page: ThreadPage<T>; issue?: string }>();
  const [loading, setLoading] = useState(true);
  const visible = result?.read === read ? result : undefined;
  const load = useCallback(async (prior?: ThreadPage<T>) => {
    active.current?.abort();
    const request = new AbortController();
    active.current = request;
    setLoading(true);
    try {
      const { value } = await read(prior?.next_cursor ?? undefined, request.signal);
      if (!request.signal.aborted) {
        const page = prior ? { ...value, items: [...prior.items, ...value.items] } : value;
        setResult({ read, page });
      }
    } catch (error) {
      if (!request.signal.aborted) {
        const canKeepPrior = !(error instanceof ClientError && [401, 403, 404, 409].includes(error.status));
        setResult({ read, page: canKeepPrior && prior ? prior : { items: [], next_cursor: null }, issue: errorText(error) });
      }
    } finally {
      if (!request.signal.aborted) setLoading(false);
    }
  }, [read]);
  useFocusEffect(useCallback(() => {
    void load();
    return () => active.current?.abort();
  }, [load, refresh]));
  return {
    items: visible?.page.items ?? [],
    issue: visible?.issue,
    loading,
    reload: () => void load(),
    more: visible?.page.next_cursor ? () => void load(visible.page) : undefined,
  };
}

export function AgentThreads({ agentId, openThread }: { agentId: string; openThread: (id: string) => void }) {
  const { threadClient } = useStore();
  return threadClient ? <AgentThreadList agentId={agentId} client={threadClient} openThread={openThread} /> : null;
}

type ThreadClient = NonNullable<ReturnType<typeof useStore>['threadClient']>;

function AgentThreadList({ agentId, client, openThread }: { agentId: string; client: ThreadClient; openThread: (id: string) => void }) {
  const { threadRefresh } = useStore();
  const read = useCallback((cursor: string | undefined, signal: AbortSignal) => client.threadList({ agent_id: agentId, cursor, limit: 25 }, { signal }), [agentId, client]);
  const page = useThreadPage<ThreadSummary>(read, threadRefresh);
  const [show, setShow] = useState(false);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const { height } = useWindowDimensions();
  const { palette } = useTextAppearance();
  if (!page.items.length && !page.issue && !page.more) return null;
  return <View style={{ backgroundColor: palette.mantle }}>
    <Pressable accessibilityRole="button" accessibilityState={{ expanded: show }} onPress={() => setShow(previous => !previous)}
      style={{ flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 12, paddingVertical: 6 }}>
      <T numberOfLines={1} style={{ flex: 1 }}><T bold color={theme.accent}>Threads</T>{page.items[0] ? <T dim> · {page.items[0].title}</T> : null}</T>
      <T dim>{show ? '▾ hide' : '▸ show'}</T>
    </Pressable>
    {show ? <ScrollView style={{ maxHeight: Math.round(height * 0.4), flexGrow: 0 }} contentContainerStyle={{ paddingHorizontal: 12, paddingBottom: 8 }}>
      {page.items.map(thread => {
        const open = expanded.has(thread.id);
        return <View key={thread.id}>
          <Pressable
            accessibilityRole="button"
            accessibilityState={{ expanded: open }}
            accessibilityLabel={`${thread.title}, ${thread.state} thread`}
            onPress={() => setExpanded(previous => {
              const next = new Set(previous);
              if (next.has(thread.id)) next.delete(thread.id);
              else next.add(thread.id);
              return next;
            })}
            style={{ minHeight: 36, flexDirection: 'row', alignItems: 'center', gap: 6 }}
          >
            <T numberOfLines={1} color={theme.accent} style={{ flexShrink: 1 }}>{open ? '▾' : '▸'} {thread.title}</T>
            <T dim>· {thread.state}</T>
          </Pressable>
          {open ? <ThreadBody client={client} thread={thread} onOpen={() => openThread(thread.id)} /> : null}
        </View>;
      })}
      {page.issue ? <T color={theme.waiting}>{page.issue}</T> : null}
      {page.issue || page.more ? <Button label={page.issue ? 'Refresh threads' : 'More threads'} onPress={page.issue ? page.reload : page.more!} disabled={page.loading} /> : null}
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
  const read = useCallback(async (_cursor: string | undefined, signal: AbortSignal) => {
    const { value } = await client.threadGet(id, { signal });
    return { value: { items: [value], next_cursor: null } };
  }, [client, id]);
  const page = useThreadPage<ThreadSummary>(read, threadRefresh);
  const thread = page.items[0];
  return <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ padding: 12, gap: 8 }}>
    {thread ? <>
      <T bold>{thread.title}<T dim> · {thread.state}</T></T>
      <ThreadBody client={client} thread={thread} />
    </> : page.issue ? <>
      <T color={theme.waiting}>{page.issue}</T>
      <Button label="Refresh thread" onPress={page.reload} disabled={page.loading} />
    </> : <T dim>Loading thread…</T>}
  </ScrollView>;
}

function eventLine(event: ThreadTimelineEvent): string {
  switch (event.kind) {
    case 'thread.renamed': return `renamed “${event.previous_title}” to “${event.title}”`;
    case 'thread.closed': return 'closed the thread';
    case 'thread.reopened': return 'reopened the thread';
    default: return 'updated the thread';
  }
}

// The same thread view renders inside a conversation and on its own screen.
function ThreadBody({ client, thread, onOpen }: { client: ThreadClient; thread: ThreadSummary; onOpen?: () => void }) {
  const { actions, threadActions, status, threadRefresh, actor } = useStore();
  const read = useCallback((cursor: string | undefined, signal: AbortSignal) => client.threadTimeline(thread.id, { cursor, limit: 25 }, { signal }), [client, thread.id]);
  const page = useThreadPage<ThreadTimelineEvent>(read, threadRefresh);
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
    {page.items.map(event => {
      const name = actor === event.actor_id || actor?.startsWith(`${event.actor_id}/`) ? 'You' : event.actor_id;
      const age = ago(new Date(event.accepted_at_unix_ms).toISOString());
      return <T key={event.id} dim><T bold>{name}</T> {eventLine(event)} · {age} ago</T>;
    })}
    {page.issue ? <T color={theme.waiting}>{page.issue}</T> : !page.items.length ? <T dim>{page.loading ? 'Loading history…' : 'No thread events yet.'}</T> : null}
    {editResult?.how === 'failed' ? <T color={theme.waiting}>{editResult.error}</T> : null}
    {editResult?.how === 'unconfirmed' ? <T color={theme.waiting}>The original connection has not confirmed this change.</T> : null}
    <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
      {threadActions.has('thread.rename') ? <Button label="Rename" onPress={rename} disabled={!editable} /> : null}
      {threadActions.has(thread.state === 'open' ? 'thread.close' : 'thread.reopen') ? <Button label={thread.state === 'open' ? 'Close thread' : 'Reopen thread'} disabled={!editable} onPress={() => void runEdit(() => actions.editThread(thread, { kind: thread.state === 'open' ? 'close' : 'reopen' }))} /> : null}
      {onOpen ? <Button label="Open thread" onPress={onOpen} /> : null}
      {page.issue || page.more ? <Button label={page.issue ? 'Refresh history' : 'Older events'} onPress={page.issue ? page.reload : page.more!} disabled={page.loading} /> : null}
      {editResult?.how === 'unconfirmed' ? <Button label="Retry original change" onPress={() => void runEdit(editResult.retry)} disabled={editing || status !== 'online'} /> : null}
    </View>
  </View>;
}
