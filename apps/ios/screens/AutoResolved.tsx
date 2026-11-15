import { useMemo } from 'react';
import { ActionSheetIOS, Alert, View } from 'react-native';
import { agentName } from '../agentsView';
import { AUTO_RESOLVED_TITLE, autoResolvedRows, OVERRIDE_TITLE, overrideCopy, type AutoResolvedRow } from '../autoResolved';
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

/** The override as an action sheet: the copy every client shows, then the ask's other named answers. One tap sends. */
export function useOverride() {
  const { status, busy, actions } = useStore();
  return (row: AutoResolvedRow) => {
    if (status !== 'online') { Alert.alert('Not connected', 'Reconnect to Fractal to answer this.'); return; }
    if (busy) return;
    const { choices } = row;
    ActionSheetIOS.showActionSheetWithOptions({ title: OVERRIDE_TITLE, message: overrideCopy(row.asker), options: [...choices.map(choice => choice.label), 'Cancel'], cancelButtonIndex: choices.length }, index => {
      if (index < choices.length) void actions.override(row.ask, choices[index].id);
    });
  };
}

// Asks Fractal resolved by their deadline while the person was away, below what waits on Now: each with
// the default that applied. While its window is open, a tap gives the person's own answer instead.
export function AutoResolvedSection({ rows }: { rows: AutoResolvedRow[] }) {
  const { autoResolved } = useStore();
  const override = useOverride();
  if (!rows.length && !autoResolved.issue) return null;
  return <View>
    <GroupedHeader title={AUTO_RESOLVED_TITLE} count={rows.length} />
    {autoResolved.issue ? <Note tone="warning">{autoResolved.issue}</Note> : null}
    {rows.map(row => row.choices.length
      ? <ContextMenu key={row.key} title={row.title} actions={[{ id: 'override', title: OVERRIDE_TITLE, symbol: 'arrow.uturn.backward.circle', run: () => override(row) }]}>
        <GroupedRow title={row.title} context={row.context} hint="Replaces the default with your answer" onPress={() => override(row)} />
      </ContextMenu>
      : <GroupedRow key={row.key} title={row.title} context={row.context} />)}
    {autoResolved.complete ? null : <Note>Older auto-resolved asks exist beyond these.</Note>}
  </View>;
}
