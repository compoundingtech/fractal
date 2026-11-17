import { useCallback, useMemo, useState, useSyncExternalStore, type ComponentType } from 'react';
import { Linking, Pressable, View } from 'react-native';
import { useFocusEffect, useNavigation } from '@react-navigation/native';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { FractalClient, HistoricalReference, PayloadUnavailable, RequestHistoryItem, RequestHistoryPage } from '../../../clients/typescript/fractal-client';
import type { StackParams } from '../navigation';
import { RequestHistoryReader, requestHistoryKey, requestHistoryRelations, requestHistoryRows } from '../requestHistory';
import { useStore } from '../store';
import { theme } from '../theme';
import { Button, Markdown, Note, openDocumentLink, T, useTextAppearance } from '../ui';

const UNAVAILABLE: Record<PayloadUnavailable, string> = {
  'retained-definition-unavailable': 'The retained request definition is unavailable.',
  'retained-step-unavailable': 'The retained step is unavailable.',
  'opening-unavailable': 'The request as it was opened is unavailable.',
  'historical-interpolation-unavailable': 'The request wording as it was shown is unavailable.',
  'boundary-unavailable': 'This earlier request is unavailable.',
  'payload-unavailable': 'The request text is unavailable.',
  'read-bound-exceeded': 'The request is too large to show here.',
};

/** Offers a new Start on one retained request's exact source episode. Threads supplies it, so this
 * screen never imports the Start flow; the row passes only the retained key and its title.
 */
export type HistoryStartAction = ComponentType<{ episode: RequestHistoryItem['key']; title: string }>;

/** Retained requests related to one thread. Read only on an explicit expand, one bounded page at
 * a time, exactly as the server returned it: no current request text stands in for a missing one.
 * Collapsing or leaving the screen releases the page; reopening reads it again.
 */
export function RequestHistory({ client, threadId, StartAction }: {
  client: Pick<FractalClient, 'threadRequestHistory'>; threadId: string; StartAction?: HistoryStartAction;
}) {
  const reader = useMemo(() => new RequestHistoryReader(client, threadId), [client, threadId]);
  const snapshot = useSyncExternalStore(reader.subscribe, reader.getSnapshot);
  const [open, setOpen] = useState<RequestHistoryReader>();
  const expanded = open === reader;
  useFocusEffect(useCallback(() => () => {
    reader.close();
    setOpen(undefined);
  }, [reader]));
  const toggle = () => {
    if (expanded) {
      reader.close();
      setOpen(undefined);
      return;
    }
    setOpen(reader);
    void reader.load();
  };
  return <View style={{ gap: 6 }}>
    <Pressable accessibilityRole="button" accessibilityState={{ expanded }} accessibilityValue={{ text: expanded ? 'Expanded' : 'Collapsed' }}
      accessibilityHint={expanded ? 'Hides earlier requests related to this thread' : 'Loads earlier requests related to this thread'}
      onPress={toggle} style={{ minHeight: 44, justifyContent: 'center' }}>
      <T bold color={theme.accent}>{expanded ? '▾' : '▸'} Request history</T>
    </Pressable>
    {expanded ? <T dim>History only · requests remain independent</T> : null}
    {!expanded ? null
      : snapshot.kind === 'loading' ? <T dim>Loading request history…</T>
      : snapshot.kind === 'error' ? <View style={{ gap: 6 }}>
        <T color={theme.waiting}>{snapshot.message}</T>
        <Button label="Retry" accessibilityLabel="Retry loading request history" onPress={() => void reader.load()} />
      </View>
      : <HistoryPage page={snapshot.page} loadingMore={snapshot.loadingMore} issue={snapshot.issue} reload={() => void reader.load()} more={() => void reader.more()} StartAction={StartAction} />}
  </View>;
}

function HistoryPage({ page, loadingMore, issue, reload, more, StartAction }: {
  page: RequestHistoryPage; loadingMore: boolean; issue?: string; reload: () => void; more: () => void; StartAction?: HistoryStartAction;
}) {
  const rows = requestHistoryRows(page);
  const byKey = new Map(rows.map(item => [requestHistoryKey(item.key), item]));
  const boundary = new Set(page.boundary_items.map(item => requestHistoryKey(item.key)));
  const relations = requestHistoryRelations(page);
  // Grouping is the server's source metadata; rows the groups do not name follow in page order.
  const grouped = new Set<string>();
  const groups = page.source_episodes.map(group => ({
    sourceId: group.source_id,
    episodes: group.episodes.length,
    items: group.episodes.flatMap(key => {
      const item = byKey.get(requestHistoryKey(key));
      if (!item) return [];
      grouped.add(requestHistoryKey(key));
      return [item];
    }),
  }));
  const ungrouped = rows.filter(item => !grouped.has(requestHistoryKey(item.key)));
  const label = (key: RequestHistoryItem['key']) => {
    const item = byKey.get(requestHistoryKey(key));
    return item?.payload.availability === 'available' && item.payload.title ? item.payload.title : `${key.source_id} · episode ${key.episode}`;
  };
  const row = (item: RequestHistoryItem) => <HistoryRow key={requestHistoryKey(item.key)} item={item} boundary={boundary.has(requestHistoryKey(item.key))} StartAction={StartAction}
    title={label(item.key)}
    follows={relations.filter(relation => requestHistoryKey(relation.request) === requestHistoryKey(item.key)).map(relation => ({ predecessor: label(relation.predecessor), reason: relation.reason }))} />;
  return <View style={{ gap: 10 }}>
    {page.source_support === 'unsupported-source' ? <Note>This thread's source does not keep request history.</Note> : null}
    {!page.complete ? <Note tone="warning">Some related requests could not be read. This page is incomplete.</Note> : null}
    {!rows.length ? <T dim>No earlier requests.</T> : null}
    {groups.map(group => group.items.length ? <View key={group.sourceId} style={{ gap: 8 }}>
      <T dim selectable>{group.sourceId} · {group.episodes === 1 ? '1 episode' : `${group.episodes} episodes`}</T>
      {group.items.map(row)}
    </View> : null)}
    {ungrouped.map(row)}
    {issue ? <Note tone="warning">{issue}</Note> : null}
    <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
      {page.next_cursor ? <Button label="Next page" hint="Replaces this page with the next requests" disabled={loadingMore} onPress={more} /> : null}
      <Button label="Refresh history" disabled={loadingMore} onPress={reload} />
    </View>
  </View>;
}

function HistoryRow({ item, boundary, follows, title, StartAction }: {
  item: RequestHistoryItem; boundary: boolean; follows: readonly { predecessor: string; reason: string }[]; title?: string; StartAction?: HistoryStartAction;
}) {
  const { data } = useStore();
  const appearance = useTextAppearance();
  const navigation = useNavigation<NativeStackNavigationProp<StackParams>>();
  const { payload, state, key } = item;
  // Only a currently active card for this exact episode proves the navigation target.
  // Cached closed cards can outlive later episodes and must never be labelled current.
  const current = data.attention.find(candidate => candidate.source_id === key.source_id
    && candidate.episode === key.episode && candidate.state === 'open' && !candidate.closedElsewhere);
  return <View style={{ gap: 4, paddingLeft: 10, borderLeftWidth: 2, borderLeftColor: appearance.color(theme.surface1) }}>
    {payload.availability === 'available'
      ? <>
        {payload.title ? <T bold selectable>{payload.title}</T> : <T dim>The original title is unavailable.</T>}
        {payload.text ? <Markdown text={payload.text} color={theme.subtext0} /> : <T dim>This request has no text.</T>}
      </>
      : <T dim>{UNAVAILABLE[payload.reason]}</T>}
    <T dim selectable>episode {key.episode}{boundary ? ' · boundary' : ''}{item.complete ? '' : ' · incomplete'}</T>
    {state.availability === 'available'
      ? <>
        <T dim>status {state.status}</T>
        {state.answers.map((answer, index) => <T key={index} dim selectable>answer {JSON.stringify(answer)}</T>)}
      </>
      : <T dim>Answer state unavailable.</T>}
    {follows.map((relation, index) => <T key={index} dim>follows {relation.predecessor} · {relation.reason}</T>)}
    {payload.availability === 'available' ? payload.references.map((reference, index) => <ReferenceLine key={index} reference={reference} />) : null}
    {current ? <Button label="Open current source" hint="Opens this exact episode's current Inbox item"
      onPress={() => navigation.navigate('Attention', { id: current.id })} /> : null}
    {/* A new Start uses only this retained key; the server's StartContext names its recipient. */}
    {StartAction ? <StartAction episode={key} title={title ?? `${key.source_id} · episode ${key.episode}`} /> : null}
  </View>;
}

/** Retained references name the revision the request held; live ones follow the target now.
 * Either way a document opens at its current version, so the tag says which one was meant.
 */
function ReferenceLine({ reference }: { reference: HistoricalReference }) {
  const retained = reference.semantics === 'retained-reference';
  const tag = retained ? `retained reference${reference.revision ? ` · revision ${reference.revision}` : ''}` : 'live reference';
  const document = reference.target_id.startsWith('doc/');
  // Only targets the existing openers support become links; no URL is built from an ID.
  const external = /^https?:\/\//i.test(reference.target_id);
  return <View style={{ gap: 2 }}>
    <T dim selectable>{reference.target_id} · {tag}</T>
    {document ? <Button label={retained ? 'Open current version' : 'Open document'} hint={retained ? 'Opens the document as it is now, not the retained revision' : undefined}
      onPress={() => openDocumentLink(reference.target_id)} />
      : external ? <Button label="Open link" hint={retained ? 'Opens the address as it is now, not the retained revision' : undefined}
        onPress={() => void Linking.openURL(reference.target_id)} /> : null}
  </View>;
}
