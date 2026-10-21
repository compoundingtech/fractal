import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { FlatList, ScrollView, View } from 'react-native';
import {
  documentListPlaceholder, emptyDocumentList, loadingDocument, parseDocumentReference,
  type DocumentList, type DocumentRow, type DocumentView,
} from '@smalltalk/st3-views';
import type { RootScreen } from '../navigation';
import { Banners, StatusLine } from '../chrome';
import { changedAt, DOCUMENTS_NOT_SERVED, documentsServed, followDocument, nextDocumentPage, openDocument, versionLine } from '../documents';
import { useStore } from '../store';
import { theme } from '../theme';
import { Button, Field, ListRow, Markdown, Note, Screen, SectionHeader, T } from '../ui';

// Documents are read-only on the phone: listed, opened and followed, never edited or published.
// Each read goes through the client's one path to a document (docs/clients/ui-contract.md#documents).

/** A row opens its latest version by name, so links to that name lead back to it on the trail. */
const rowReference = (row: DocumentRow) => (row.latest ? row.name : row.reference);

export function DocumentsScreen({ route, navigation }: RootScreen<'Documents'>) {
  const { client, status, caps } = useStore();
  const served = documentsServed(caps);
  const reader = status === 'online' && served !== false ? client : null;
  const [draft, setDraft] = useState(route.params?.prefix ?? '');
  const [prefix, setPrefix] = useState<string | null>(route.params?.prefix ?? null);
  const [list, setList] = useState<DocumentList>(() => emptyDocumentList(prefix));
  const reading = useRef<AbortController | null>(null);
  const read = useCallback((from: DocumentList) => {
    reading.current?.abort();
    const abort = new AbortController();
    reading.current = abort;
    void nextDocumentPage(reader, from, abort.signal).then(next => { if (!abort.signal.aborted) { reading.current = null; setList(next); } });
  }, [reader]);
  useEffect(() => {
    const fresh = emptyDocumentList(prefix);
    setList(fresh);
    read(fresh);
    return () => reading.current?.abort();
  }, [prefix, read]);
  const typed = draft.trim();
  const openable = parseDocumentReference(typed) !== null;
  const listable = typed === '' || typed.startsWith('doc/');
  const open = (reference: string) => navigation.push('Document', { reference, trail: [reference] });
  const submit = () => { if (openable) open(typed); else if (listable) setPrefix(typed || null); };
  const placeholder = documentListPlaceholder(list);
  return <Screen><Banners />
    <FlatList
      contentInsetAdjustmentBehavior="automatic"
      keyboardShouldPersistTaps="handled"
      data={list.rows}
      keyExtractor={row => row.reference}
      ListHeaderComponent={<>
        <StatusLine />
        <View style={{ padding: 12 }}>
          <Field accessibilityLabel="Document name or prefix" value={draft} onChangeText={setDraft} onSubmitEditing={submit} placeholder="doc/NAME, or a prefix such as doc/garden/" autoCapitalize="none" autoCorrect={false} spellCheck={false} returnKeyType="go" />
          <View style={{ flexDirection: 'row', gap: 8 }}>
            <Button label="Open" disabled={!openable} onPress={() => open(typed)} />
            <Button label={typed ? 'List under this prefix' : 'List all'} disabled={!listable} onPress={() => setPrefix(typed || null)} />
          </View>
          {typed && !openable && !listable ? <Note>A document name starts with doc/.</Note> : null}
        </View>
        <SectionHeader title={prefix ? `under ${prefix}` : 'documents'} count={list.rows.length || undefined} />
      </>}
      renderItem={({ item: row }) => <ListRow
        glyph="▤"
        glyphColor={theme.lavender}
        title={row.name}
        second={[row.shortVersion, changedAt(row.changedAtUnixMs) ? `changed ${changedAt(row.changedAtUnixMs)}` : null].filter(Boolean).join(' · ')}
        onPress={() => open(rowReference(row))}
      />}
      onEndReachedThreshold={0.5}
      onEndReached={() => { if (list.nextCursor && !list.failed && !reading.current) read(list); }}
      ListFooterComponent={<View style={{ paddingBottom: 32 }}>
        {served === false ? <Note tone="warning">{DOCUMENTS_NOT_SERVED}</Note>
          : placeholder ? <Note tone={list.failed ? 'warning' : 'dim'}>{placeholder}</Note> : null}
        {list.failed && list.rows.length ? <Note tone="warning">More documents could not be listed.</Note> : null}
        {list.failed ? <View style={{ paddingHorizontal: 12 }}><Button label="Try again" onPress={() => read(list)} /></View> : null}
        {list.nextCursor && !list.failed ? <Note>Loading more…</Note> : null}
      </View>}
    />
  </Screen>;
}

export function DocumentScreen({ route, navigation }: RootScreen<'Document'>) {
  const { client, status, caps } = useStore();
  const served = documentsServed(caps);
  const reader = status === 'online' && served !== false ? client : null;
  const { reference, trail } = route.params;
  const [view, setView] = useState<DocumentView>(() => loadingDocument(reference));
  const [attempt, setAttempt] = useState(0);
  const shown = useRef(view);
  shown.current = view;
  useEffect(() => {
    // Going offline keeps a document already read on screen; it is read again once online.
    if (!reader && shown.current.state === 'ready') return;
    const abort = new AbortController();
    if (shown.current.state !== 'ready') setView(loadingDocument(reference));
    void openDocument(reader, reference, abort.signal).then(next => { if (!abort.signal.aborted) setView(next); });
    return () => abort.abort();
  }, [reference, reader, attempt]);
  useLayoutEffect(() => {
    navigation.setOptions({ title: (view.state === 'ready' ? view.name : reference).replace(/^doc\//, '') });
  }, [navigation, view, reference]);
  const follow = useCallback((next: string) => {
    const step = followDocument(trail, next);
    if (step.kind === 'push') navigation.push('Document', { reference: next, trail: step.trail });
    else if (step.kind === 'back') navigation.pop(step.count);
  }, [navigation, trail]);
  const from = trail.length > 1 ? trail[trail.length - 2] : null;
  return <Screen><Banners />
    <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 32 }}>
      <StatusLine />
      {from ? <View style={{ paddingHorizontal: 12 }}><Button label={`‹ Back to ${from.replace(/^doc\//, '')}`} onPress={() => navigation.goBack()} /></View> : null}
      {view.state === 'loading' ? <Note>Loading {reference}…</Note> : null}
      {view.state === 'unavailable' ? <View style={{ padding: 12, gap: 4 }}>
        <T bold selectable>{reference}</T>
        <T color={theme.waiting}>{served === false ? DOCUMENTS_NOT_SERVED : view.explanation}</T>
        {view.why !== 'not-text' ? <Button label="Try again" onPress={() => setAttempt(value => value + 1)} /> : null}
      </View> : null}
      {view.state === 'ready' ? <>
        <View style={{ padding: 12, gap: 2 }}>
          <T bold selectable>{view.name}</T>
          <T dim selectable>{versionLine(view)}</T>
          {view.owner ? <T dim>by {view.owner}</T> : null}
        </View>
        <View style={{ paddingHorizontal: 12 }}>
          {view.text ? <Markdown text={view.text} onDocument={follow} /> : <Note>This document is empty.</Note>}
        </View>
        {view.links.length ? <>
          <SectionHeader title="links" count={view.links.length} />
          {view.links.map(link => <ListRow key={link.reference} glyph="→" glyphColor={theme.lavender} title={link.name} second={link.version ? `version ${link.version.slice(0, 12)}` : 'latest version'} onPress={() => follow(link.reference)} />)}
        </> : null}
      </> : null}
    </ScrollView>
  </Screen>;
}
