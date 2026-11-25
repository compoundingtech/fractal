import { useSyncExternalStore } from 'react';
import { Alert, View } from 'react-native';
import * as Clipboard from 'expo-clipboard';
import * as Crypto from 'expo-crypto';
import { Button, T } from './ui';
import { useStore } from './store';
import type { OutboxRecoveryRow } from './outboxMessage.ts';

export const confirmRecoveryDiscard = (discard: () => void): void => Alert.alert('Discard local recovery?',
  'This deletes local recovery only. It does not cancel the message or prove it was not applied. The original may already have arrived.', [
    { text: 'Cancel', style: 'cancel' },
    { text: 'Discard', style: 'destructive', onPress: discard },
  ]);

const empty: readonly { draft: OutboxRecoveryRow }[] = [];
const noSubscribe = () => () => {};
const emptySnapshot = () => empty;
export function OutboxRecovery({ chat, thread, starts, onOpen }: {
  chat?: string; thread?: string; starts?: boolean;
  onOpen: (target: string, title: string) => void;
}) {
  const { outbox, outboxIssue, retryOutbox } = useStore();
  const entries = useSyncExternalStore(outbox?.attempts.subscribe ?? noSubscribe, outbox?.attempts.getSnapshot ?? emptySnapshot);
  const rows = entries.map(entry => entry.draft).filter(row => {
    const message = row.message;
    return !message || (chat ? message.kind === 'chat' && message.to === chat
      : starts ? message.kind === 'start' : thread && message.kind !== 'chat' && message.target === thread);
  });
  const issue = outboxIssue || outbox?.issue;
  return <View style={{ gap: 8 }}>
    {issue ? <View><T dim>Storage unavailable; retry later. {issue}</T><Button label="Retry storage later" onPress={retryOutbox} /></View> : null}
    {rows.map(row => <View key={row.id} style={{ gap: 4, padding: 8 }}>
      <T bold>{row.message?.kind === 'start' ? row.message.title : 'Message recovery'}</T>
      <T dim>{row.member} · {row.status}</T>
      {row.text !== undefined ? <T selectable>{row.text}</T> : null}
      {row.images ? <T dim>{row.images} privately stored image{row.images === 1 ? '' : 's'}</T> : null}
      {row.state === 'not-applied' ? <Button label="Retry as new attempt" onPress={() => { void outbox?.retry(row.id, `action/ios-${Crypto.randomUUID()}`); }} /> : null}
      {row.canCheck ? <Button label="Check status" onPress={() => { void outbox?.checkStatus(row.id); }} /> : null}
      {row.message ? <Button label="Open conversation" onPress={() => {
        const message = row.message!;
        onOpen(message.kind === 'chat' ? message.to : message.target, message.kind === 'start' ? message.title : 'Conversation');
      }} /> : null}
      {row.state !== 'corrupt' && row.text !== undefined ? <Button label="Copy message" onPress={() => { void Clipboard.setStringAsync(row.text!); }} /> : null}
      <Button label="Discard recovery" onPress={() => confirmRecoveryDiscard(() => { void outbox?.discard(row.id); })} />
    </View>)}
  </View>;
}
