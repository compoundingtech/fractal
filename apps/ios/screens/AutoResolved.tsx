import { useMemo } from 'react';
import { View } from 'react-native';
import { showOwnedActionSheet, OwnedAlert as Alert } from '../ownedPresentationNative';
import { agentName } from '../agentsView';
import { AUTO_RESOLVED_TITLE, autoResolvedRows, autoResolvedVisible, INCOMPLETE_AUTO_RESOLVED, needsWords, OVERRIDE_TITLE, overrideAnswer, overrideCopy, type AutoResolvedRow, type OverrideChoice } from '../autoResolved';
import { ContextMenu } from '../menu';
import { useStore } from '../store';
import { GroupedHeader, GroupedRow, Note } from '../ui';

/** The auto-resolved rows, the asker named as the copy names them: the agent's name, or its ID. */
export function useAutoResolvedRows(): AutoResolvedRow[] {
  const { autoResolved, data } = useStore();
  return useMemo(() => autoResolvedRows(autoResolved.items, id => {
    const agent = data.agents.find(candidate => candidate.id === id);
    return agent ? agentName(agent) : id;
  }), [autoResolved.items, data.agents]);
}

/**
 * The override as an action sheet: the copy every client shows, then the ask's other named answers. One tap
 * sends; an answer that requests changes first asks what should change, as answering on Now does. Nothing is
 * offered while an override of the same ask is in flight.
 */
export function useOverride() {
  const { status, busy, actions, overridePending } = useStore();
  const send = (row: AutoResolvedRow, choice: OverrideChoice, words?: string) => {
    const answer = overrideAnswer(choice, words);
    if (typeof answer === 'string') Alert.alert(choice.label, answer);
    else void actions.override(row.ask, answer);
  };
  return (row: AutoResolvedRow) => {
    if (status !== 'online') { Alert.alert('Not connected', 'Reconnect to Fractal to answer this.'); return; }
    if (busy || overridePending.has(row.ask.episode)) return;
    const { choices } = row;
    showOwnedActionSheet({ title: OVERRIDE_TITLE, message: overrideCopy(row.asker), options: [...choices.map(choice => choice.label), 'Cancel'], cancelButtonIndex: choices.length }, index => {
      const choice = choices[index];
      if (!choice) return;
      if (needsWords(choice)) Alert.prompt(choice.label, 'What should change?', text => send(row, choice, text));
      else send(row, choice);
    });
  };
}

// Asks Fractal resolved by their deadline while the person was away, below what waits on Now: each with
// the default that applied. While its window is open, a tap gives the person's own answer instead.
export function AutoResolvedSection({ rows }: { rows: AutoResolvedRow[] }) {
  const { autoResolved, overridePending } = useStore();
  const override = useOverride();
  if (!autoResolvedVisible(rows, autoResolved)) return null;
  return <View>
    <GroupedHeader title={AUTO_RESOLVED_TITLE} count={rows.length} />
    {autoResolved.issue ? <Note tone="warning">{autoResolved.issue}</Note> : null}
    {rows.map(row => overridePending.has(row.ask.episode)
      ? <GroupedRow key={row.key} title={row.title} context={`${row.context} · Sending your answer…`} />
      : row.choices.length
        ? <ContextMenu key={row.key} title={row.title} actions={[{ id: 'override', title: OVERRIDE_TITLE, symbol: 'arrow.uturn.backward.circle', run: () => override(row) }]}>
          <GroupedRow title={row.title} context={row.context} hint="Replaces the default with your answer" onPress={() => override(row)} />
        </ContextMenu>
        : <GroupedRow key={row.key} title={row.title} context={row.context} />)}
    {autoResolved.complete ? null : <Note tone="warning">{INCOMPLETE_AUTO_RESOLVED}</Note>}
  </View>;
}
