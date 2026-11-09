import { useCallback, useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import { Alert, Keyboard, KeyboardAvoidingView, Linking, Pressable, ScrollView, SectionList, TextInput, View } from 'react-native';
import { useFocusEffect, useNavigation } from '@react-navigation/native';
import { useHeaderHeight } from '@react-navigation/elements';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import { Banners, Empty, StatusLine, useDebugScroll, useMissionsOnFocus, useRefresh } from '../chrome';
import { ContextMenu } from '../menu';
import { alertsHeading, detailRow, homeRows, homeSections, openAlerts, type HomeRow, cleanMessageText, ANSWERS, isRequest, spaced } from '@fractal/views';
import type { RootParams } from '../navigation';
import { agentName } from '../agentsView';
import { alertsEmptyText } from '../alertsEmpty';
import { attentionActionLabel, attentionKindLabel, blockedLine } from '../presentation';
import { missionTitle } from '../missionsView';
import { useStore } from '../store';
import { randomName } from '../launcher';
import { AlertBody, RequestAnswers, RequestQuestion, StructuredRequestView } from './Alerts';
import { ThreadViewport } from '../threadViewport';
import { InboxThread, PendingInboxThreads, ThreadGroup } from './Threads';
import { theme } from '../theme';
import { Button, GroupedHeader, GroupedRow, Markdown, Note, SystemScreen as Screen, T } from '../ui';
import type { RootScreen } from '../navigation';

// Now: what needs the person, as the Fractal terminal UI's Home lists it.
export function NowScreen() {
  const { data, caps, hasSynced, truncated, loadErrors, busy, status, sync, actions } = useStore();
  const navigation = useNavigation<NativeStackNavigationProp<RootParams>>();
  const list = useRef<SectionList<HomeRow>>(null);
  const threadViewport = useMemo(() => ({ get current() { return list.current?.getScrollResponder() ?? null; } }), []);
  useDebugScroll(list as never);
  const refresh = useRefresh();
  const agentOf = (row: HomeRow) => row.item.conversation_id?.startsWith('agent/') ? row.item.conversation_id : undefined;
  const rows = useMemo(() => homeRows(data.attention, caps?.session_actor), [data.attention, caps?.session_actor]);
  const requesterOf = (row: HomeRow) => {
    const id = row.item.requester_id ?? row.item.source_id;
    const agent = data.agents.find(candidate => candidate.id === id);
    return agent ? agentName(agent) : id.replace(/^agent\//, '');
  };
  // Start something, as the Fractal terminal UI's launcher does: a shell or a mission.
  useLayoutEffect(() => {
    navigation.setOptions({
      unstable_headerRightItems: () => [{
        type: 'menu', label: 'New', icon: { type: 'sfSymbol', name: 'plus' },
        menu: { items: [
          { type: 'action', label: 'New terminal', icon: { type: 'sfSymbol', name: 'terminal' }, onPress: () => void actions.createTerminal(randomName()).then(terminalId => { if (terminalId) navigation.navigate('Terminal', { terminalId, title: 'shell' }); }) },
          { type: 'action', label: 'New mission', icon: { type: 'sfSymbol', name: 'point.3.connected.trianglepath.dotted' }, onPress: () => navigation.navigate('NewMission') },
        ] },
      }],
    });
  }, [navigation, actions]);
  // The count of what waits on the person, as the other clients print it: nothing at zero.
  const heading = alertsHeading(openAlerts(data.attention, caps?.session_actor).length);
  const sections = useMemo(() => homeSections(rows).map(section => ({ ...section, data: section.rows })), [rows]);
  // Dismiss, as the Fractal terminal UI's x: a request is closed with word to its asker that there is nothing for
  // the person to do (after a confirm), an update or a message is read. A decision has none.
  const dismiss = (row: HomeRow): (() => void) | undefined => {
    const item = row.item;
    // Closed elsewhere: only clearing it takes it off Now.
    if (item.closedElsewhere) return () => actions.clearClosed(item.id);
    if (item.update && item.actions.includes('work.done')) return () => void actions.done(item, 'Read', 'read');
    if (isRequest(item.attention_kind) && item.actions.includes('work.done')) {
      const asker = item.requester_id?.replace(/^agent\//, '') ?? 'the agent';
      return () => Alert.alert(`Tell ${asker} there is nothing for you to do`, row.title, [
        { text: 'Cancel', style: 'cancel' },
        { text: 'Dismiss', onPress: () => void actions.done(item, ANSWERS.nothing) },
      ]);
    }
    if (item.actions.includes('message.read')) return () => void actions.markRead(item);
    return undefined;
  };
  return <Screen>
    <Banners />
    <SectionList
      ref={list}
      contentInsetAdjustmentBehavior="automatic"
      sections={sections}
      keyExtractor={row => row.item.id}
      stickySectionHeadersEnabled={false}
      renderSectionHeader={({ section }) => <GroupedHeader title={section.title} count={section.count} />}
      renderItem={({ item: row }) => <ContextMenu title={row.title} actions={[
        { id: 'open', title: 'Open', symbol: 'arrow.up.right', run: () => navigation.navigate('Attention', { id: row.item.id }) },
        ...(agentOf(row) ? [{ id: 'chat', title: 'Chat with the agent', symbol: 'bubble.left.and.bubble.right', run: () => navigation.navigate('Conversation', { target: agentOf(row)! }) }] : []),
        ...(row.item.mission_id ? [{ id: 'mission', title: 'Open the mission', symbol: 'point.3.connected.trianglepath.dotted', run: () => navigation.navigate('Mission', { id: row.item.mission_id! }) }] : []),
        ...(row.item.actions.includes('work.done') && !row.item.update && !busy && status === 'online' ? [{ id: 'done', title: 'Complete step', symbol: 'checkmark.circle', run: () => Alert.prompt('Complete step', row.title, summary => { if (summary.trim()) void actions.done(row.item, summary); }) }] : []),
        ...(dismiss(row) && !busy && status === 'online' ? [{ id: 'dismiss', title: row.item.closedElsewhere ? 'Clear from Now' : row.item.update ? 'Mark read' : 'Dismiss', symbol: 'xmark.circle', run: dismiss(row)! }] : []),
      ]}>
        <GroupedRow
          title={row.title}
          context={[`Kind: ${row.kind}`, `From: ${requesterOf(row)}`, row.waiting, !row.item.closedElsewhere && row.item.thread_unread ? 'Unread thread replies' : undefined, `Waited ${row.age}`].filter(Boolean).join(' · ')}
          onPress={() => navigation.navigate('Attention', { id: row.item.id })}
        />
      </ContextMenu>}
      ListHeaderComponent={<View><StatusLine />{heading ? <T bold style={{ paddingHorizontal: 32, paddingTop: 16 }}>{heading}</T> : null}</View>}
      refreshControl={refresh}
      ListEmptyComponent={<Empty text={alertsEmptyText({ loadError: loadErrors.attention, status, hasSynced, sync })} />}
      ListFooterComponent={<View>
        <PendingInboxThreads onOpen={id => navigation.navigate('Thread', { id })} />
        {truncated.attention ? <Note tone="warning">More items exist beyond these 200. `fractal now` lists them all.</Note> : null}
        <ThreadViewport viewport={threadViewport}>
          <ThreadGroup title="Recently closed threads" context={{ recently_closed: true }} openThread={id => navigation.navigate('Thread', { id })} />
        </ThreadViewport>
      </View>}
      style={{ flex: 1 }}
    />
  </Screen>;
}

// One attention item: what it asks, who raised it, and what can be done here.
export function AttentionScreen({ route, navigation }: RootScreen<'Attention'>) {
  const { data, busy, status, actions, hasSynced, loadErrors } = useStore();
  useMissionsOnFocus();
  const requestScroll = useRef<ScrollView>(null);
  const headerHeight = useHeaderHeight();
  useFocusEffect(useCallback(() => {
    // The only inline text input on an Inbox request is its thread composer.
    // Reveal it at the foot of the request after the keyboard reduces the view.
    let frame: number | undefined;
    const listener = Keyboard.addListener('keyboardDidShow', () => {
      if (TextInput.State.currentlyFocusedInput()) {
        frame = requestAnimationFrame(() => requestScroll.current?.scrollToEnd({ animated: false }));
      }
    });
    return () => {
      listener.remove();
      if (frame !== undefined) cancelAnimationFrame(frame);
    };
  }, []));
  const found = data.attention.find(candidate => candidate.id === route.params.id);
  // Nothing leaves Now by itself: an update is read when the person says so, never by opening it.
  const kept = useRef(found);
  if (found) kept.current = found;
  const item = found ?? kept.current;
  if (!item) return <Screen><Banners /><Empty text={loadErrors.attention ? 'This alert could not be loaded.' : !hasSynced ? 'Checking for this alert…' : status !== 'online' ? 'This alert is not in the offline snapshot. Reconnect to check it.' : 'This item is no longer open.'} /></Screen>;
  const row = detailRow(item);
  const publisherId = [item.requester_id, item.source_id].find(id => id?.startsWith('agent/'));
  const publisher = publisherId ? data.agents.find(candidate => candidate.id === publisherId) : undefined;
  const mission = item.mission_id ? data.missions.find(candidate => candidate.id === item.mission_id) : undefined;
  const agentId = item.conversation_id?.startsWith('agent/') && item.conversation_id.length > 'agent/'.length
    ? item.conversation_id : undefined;
  const agent = agentId ? data.agents.find(candidate => candidate.id === agentId) : undefined;
  const threadEntry = <ThreadViewport viewport={requestScroll}><InboxThread sourceId={item.source_id}
    origin={item.source_id && item.episode ? { sourceId: item.source_id, episode: item.episode } : undefined}
    agentId={agentId} initialTitle={item.title} onOpen={id => navigation.navigate('Thread', { id })} /></ThreadViewport>;
  const other = item.actions.filter(action => action !== 'work.done' && action !== 'message.read' && action !== 'prompt.respond');
  // Closed elsewhere, so nothing here can be answered; it stays until the person clears it.
  if (item.closedElsewhere && !item.update) {
    return <Screen>
      <Banners />
      <ScrollView ref={requestScroll} contentInsetAdjustmentBehavior="automatic" automaticallyAdjustKeyboardInsets keyboardShouldPersistTaps="handled" contentContainerStyle={{ padding: 12, gap: 6, paddingBottom: 32 }}>
        <T><T bold color={theme[row.color]}>{row.glyph} {row.kind}</T><T dim>  {row.age} ago</T></T>
        <T bold selectable>{row.title}</T>
        {item.detail ? <Markdown text={item.detail} color={theme.subtext0} /> : null}
        <T dim>This was closed. It stays under Recently closed until you clear it.</T>
        {threadEntry}
        <Button label="clear from Now" onPress={() => { actions.clearClosed(item.id); navigation.goBack(); }} />
        <T dim selectable>{item.id}</T>
      </ScrollView>
    </Screen>;
  }
  if (item.update) {
    const from = publisher ? agentName(publisher) : item.requester_id?.replace(/^agent\//, '') ?? 'An agent';
    return <Screen>
      <Banners />
      <ScrollView ref={requestScroll} contentInsetAdjustmentBehavior="automatic" automaticallyAdjustKeyboardInsets keyboardShouldPersistTaps="handled" contentContainerStyle={{ padding: 12, gap: 8, paddingBottom: 32 }}>
        <T><T bold color={theme[row.color]}>{row.glyph} update</T><T dim>  {row.age} ago</T></T>
        <T bold selectable>{row.title}</T>
        <T><T dim>from  </T><T bold color={theme.person}>{from}</T></T>
        <Markdown text={spaced(cleanMessageText(item.detail || item.update.summary || ''))} color={theme.text} />
        <T dim selectable>about {item.update.about}</T>
        {item.update.subjects?.map((subject, index) => subject.url
          ? <Pressable key={index} onPress={() => void Linking.openURL(subject.url!)}><T color={theme.accent}>↗ {subject.label}</T></Pressable>
          : <T key={index} dim>↗ {subject.label}  {subject.ref ?? ''}</T>)}
        {threadEntry}
        <T dim>{item.closedElsewhere ? 'This was closed. It stays under Recently closed until you clear it.' : 'Nothing waits on this. It stays on Now until you mark it read.'}</T>
        {item.closedElsewhere
          ? <Button label="clear from Now" onPress={() => { actions.clearClosed(item.id); navigation.goBack(); }} />
          : item.actions.includes('work.done') ? <Button label="mark read" disabled={busy || status !== 'online'} onPress={() => void actions.done(item, 'Read', 'read').then(done => { if (done) navigation.goBack(); })} /> : null}
        {agentId ? <Button label="Open agent conversation" onPress={() => navigation.navigate('Conversation', { target: agentId, title: agent ? agentName(agent) : undefined })} /> : null}
        <T dim selectable>{item.id}</T>
      </ScrollView>
    </Screen>;
  }
  if (isRequest(item.attention_kind) && item.actions.includes('work.done')) {
    const from = publisher ? agentName(publisher) : item.requester_id?.replace(/^agent\//, '') ?? 'An agent';
    return <Screen>
      <Banners />
      {/* Automatic insets keep the end of a long request clear of the floating tab bar. */}
      <KeyboardAvoidingView style={{ flex: 1 }} behavior="padding" keyboardVerticalOffset={headerHeight}>
  <ScrollView ref={requestScroll} contentInsetAdjustmentBehavior="automatic" keyboardShouldPersistTaps="handled" contentContainerStyle={{ padding: 12, gap: 8, paddingBottom: 32 }}>
          <T><T bold color={theme[row.color]}>{row.glyph} request</T><T dim>  {item.priority} · waited {row.age}</T></T>
          <T bold selectable>{row.title}</T>
          <T><T dim>asks  </T><T bold color={theme.person}>{from}</T></T>
          {item.request ? <StructuredRequestView item={item} request={item.request} from={from} onAnswered={() => navigation.goBack()} /> : <>
            <RequestQuestion text={cleanMessageText(item.detail ?? '')} />
            <T bold color={theme.person}>{from} is waiting on you.</T>
            <RequestAnswers item={item} from={from} onAnswered={() => navigation.goBack()} />
          </>}
          {blockedLine(item) ? <T dim selectable>{blockedLine(item)}</T> : <T dim>Answer it, or Nothing to do if there is nothing for you to do. Either way the step it waits on continues.</T>}
          {mission ? <Button label={`mission ${missionTitle(mission)}`} onPress={() => navigation.navigate('Mission', { id: mission.id })} /> : item.mission_id ? <T dim>mission {item.mission_id}</T> : null}
          {threadEntry}
          {agentId ? <Button label="Open agent conversation" onPress={() => navigation.navigate('Conversation', { target: agentId, title: agent ? agentName(agent) : undefined })} /> : null}
          <T dim selectable>{item.id}</T>
        </ScrollView>
      </KeyboardAvoidingView>
    </Screen>;
  }
  return <Screen>
    <Banners />
    <ScrollView ref={requestScroll} contentInsetAdjustmentBehavior="automatic" automaticallyAdjustKeyboardInsets keyboardShouldPersistTaps="handled" contentContainerStyle={{ padding: 12, gap: 6, paddingBottom: 32 }}>
      <T><T bold color={theme[row.color]}>{row.glyph} {row.kind}</T><T dim>  {attentionKindLabel(item.attention_kind)} · {item.priority} · waited {row.age}</T></T>
      <T bold selectable>{row.title}</T>
      {item.attention_kind === 'harness-prompt' || item.attention_kind === 'harness-login'
        // A prompt is allowed from its conversation, where the call it asks about is shown.
        ? <AlertBody item={item} from={publisher ? agentName(publisher) : item.source_id} seats={[]} />
        : item.detail ? <Markdown text={item.detail} color={theme.subtext0} /> : null}
      {item.because ? <T soft>because {item.because}</T> : null}
      {row.waiting ? <T dim>{row.waiting}</T> : null}
      {threadEntry}
      {mission ? <Button label={`mission ${missionTitle(mission)}`} onPress={() => navigation.navigate('Mission', { id: mission.id })} /> : item.mission_id ? <T dim>mission {item.mission_id}</T> : null}
      {agentId ? <Button label="Open agent conversation" onPress={() => navigation.navigate('Conversation', { target: agentId, title: agent ? agentName(agent) : undefined })} /> : null}
      <T dim selectable>{item.id}{item.source_id !== item.id ? ` · from ${item.source_id}` : ''}</T>
      {item.actions.includes('work.done') ? <Button label={attentionActionLabel('work.done').toLowerCase()} disabled={busy || status !== 'online'} onPress={() => Alert.prompt('Complete step', item.title, summary => { if (summary.trim()) void actions.done(item, summary).then(done => { if (done) navigation.goBack(); }); })} /> : null}
      {item.actions.includes('message.read') ? <Button label="dismiss: mark read" disabled={busy || status !== 'online'} onPress={() => void actions.markRead(item).then(done => { if (done) navigation.goBack(); })} /> : null}
      {other.length ? <Note>in the CLI: {other.map(attentionActionLabel).join(', ')}</Note> : null}
    </ScrollView>
  </Screen>;
}
