import { View } from 'react-native';
import { availabilityRows } from '../availability';
import { useStore } from '../store';
import { GroupedHeader, GroupedRow, Note } from '../ui';

// The person's availability and directive note, at the top of Now as `fractal now` prints them.
export function AvailabilitySection() {
  const { availability } = useStore();
  const { person, note, issue } = availability;
  if (!issue && !person) return null;
  return <View>
    <GroupedHeader title="Availability" />
    {issue ? <Note tone="warning">{issue}</Note> : null}
    {person ? availabilityRows(person, note, new Date()).map(row => <GroupedRow key={row.key} title={row.title} context={row.context} />) : null}
  </View>;
}
