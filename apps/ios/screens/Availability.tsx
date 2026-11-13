import { ActionSheetIOS, Alert, View } from 'react-native';
import { availabilityRows, AXES, axisChange, CLEARED, CUSTOM_HINT, customUntil, presetUntil, REPAIR_FOR_AVAILABILITY, UNTIL_PRESETS, type AvailabilityRow, type Axis } from '../availability';
import { useStore } from '../store';
import { GroupedHeader, GroupedRow, Note } from '../ui';

const HINTS = { status: 'Sets or clears your status', interruptions: 'Sets which asks may interrupt you', deadline_clock: 'Runs or holds the deadlines of asks to you', note: 'Writes or clears your note for agents' } as const;

/** How long a change holds: a preset, a typed time or span, or until the person changes it. */
function pickEnd(title: string, open: string, done: (until: Date | null) => void) {
  const options = [...UNTIL_PRESETS, 'Custom…', open, 'Cancel'];
  ActionSheetIOS.showActionSheetWithOptions({ title, options, cancelButtonIndex: options.length - 1 }, index => {
    if (index < UNTIL_PRESETS.length) done(presetUntil(UNTIL_PRESETS[index], new Date()));
    else if (index === UNTIL_PRESETS.length) Alert.prompt('Until when?', CUSTOM_HINT, text => {
      const until = customUntil(text, new Date());
      if (typeof until === 'string') Alert.alert('Not a time', until); else done(until);
    });
    else if (index === UNTIL_PRESETS.length + 1) done(null);
  });
}

// The person's availability and directive note, at the top of Now as `fractal now` prints them.
// Tapping a row changes it: a value from an action sheet, then how long it holds.
export function AvailabilitySection() {
  const { availability, actor, caps, status, actions } = useStore();
  const { person, note, issue } = availability;
  if (!issue && person?.person !== actor) return null;
  const granted = (scope: string) => caps?.capabilities.some(capability => capability.id === scope && capability.state === 'granted') ?? false;
  const choose = (axis: Axis) => {
    const { title, choices } = AXES[axis];
    ActionSheetIOS.showActionSheetWithOptions({ title, options: [...choices.map(([label]) => label), 'Cancel'], cancelButtonIndex: choices.length }, index => {
      if (index >= choices.length) return;
      const [label, value] = choices[index];
      if (value === CLEARED[axis]) void actions.changeAvailability(axisChange(axis, value, null));
      else pickEnd(`${label} until…`, 'Until I change it', until => void actions.changeAvailability(axisChange(axis, value, until)));
    });
  };
  const writeNote = () => Alert.prompt('Note for agents', 'Agents read it before they ask you. It never assigns work or grants authority.', [
    { text: 'Cancel', style: 'cancel' },
    ...(note ? [{ text: 'Clear', style: 'destructive' as const, onPress: () => void actions.changeNote(null) }] : []),
    { text: 'Next', onPress: (text?: string) => { const kept = text?.trim(); if (kept) pickEnd('Keep the note until…', 'Until I clear it', until => void actions.changeNote(kept, until)); } },
  ], 'plain-text', note?.text);
  const edit = (row: AvailabilityRow) => {
    if (status !== 'online') Alert.alert('Not connected', 'Reconnect to Fractal to change this.');
    else if (!granted(row.key === 'note' ? 'write.notes' : 'write.availability')) Alert.alert('Re-pair to change this', REPAIR_FOR_AVAILABILITY);
    else if (row.key === 'note') writeNote();
    else choose(row.key);
  };
  return <View>
    <GroupedHeader title="Availability" />
    {issue ? <Note tone="warning">{issue}</Note> : null}
    {caps && !granted('write.availability') ? <Note>{REPAIR_FOR_AVAILABILITY}</Note> : null}
    {person?.person === actor ? availabilityRows(person, note, new Date()).map(row => <GroupedRow key={row.key} title={row.title} context={row.context} hint={HINTS[row.key]} onPress={() => edit(row)} />) : null}
  </View>;
}
