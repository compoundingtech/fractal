import assert from 'node:assert/strict';
import { test } from 'node:test';
import { availabilityRows, ownAvailability, servesAvailability, whenText } from './availability.ts';

// Local wall-clock moments, so the expectations hold in any time zone.
const now = new Date(2026, 10, 12, 14, 0);
const at = (day, hour, minute = 0) => new Date(2026, 10, day, hour, minute).toISOString();
const base = {
  person: 'person/ada', status: 'available', status_source: 'presence', status_until: null,
  presence: 'present', presence_observed_at: at(12, 13, 57), interruptions: 'all', interruptions_until: null,
  deadline_clock: 'run', deadline_clock_since: null, deadline_clock_until: null,
};
const titles = rows => rows.map(row => `${row.title} | ${row.context}`);

test('a member serves availability when it lists the write scope, granted or not', () => {
  assert.equal(servesAvailability(null), false);
  assert.equal(servesAvailability({ capabilities: [{ id: 'summary', state: 'granted', version: 1 }] }), false);
  assert.equal(servesAvailability({ capabilities: [{ id: 'write.availability', state: 'ungranted', version: 0 }] }), true);
});

test('the session person reads their own entries', () => {
  const note = { person: 'person/ada', author: 'person/ada', revision: 'r', text: 'ship it', time: at(12, 9) };
  assert.deepEqual(ownAvailability([{ ...base, person: 'person/avery' }, base], [note], 'person/ada'), { person: base, note });
  assert.deepEqual(ownAvailability([base], [], 'person/avery'), { person: null, note: null });
});

test('times read in local time near now', () => {
  assert.equal(whenText(at(12, 18), now), '18:00');
  assert.equal(whenText(at(13, 9), now), 'tomorrow 09:00');
  assert.equal(whenText(at(11, 16, 5), now), 'yesterday 16:05');
  assert.equal(whenText(at(20, 9), now), 'Nov 20 09:00');
});

test('the defaults: status follows presence, everything may ask, the clock runs, no note', () => {
  assert.deepEqual(titles(availabilityRows(base, null, now)), [
    'Available | From your devices: present, reported 3m ago',
    'Interruptions: all | Agents may ask you anything',
    'Deadline clock: running | Ask deadlines run as set',
    'No note | A note tells agents what matters to you now',
  ]);
  assert.equal(availabilityRows({ ...base, status: 'away', presence: 'unreported' }, null, now)[0].context, 'From your devices: no recent report');
  assert.equal(availabilityRows({ ...base, status: 'away', presence: 'idle', presence_observed_at: at(12, 13) }, null, now)[0].context, 'From your devices: idle, reported 1h ago');
});

test('with no device reporting the person reads as available, never away', () => {
  const unreported = { ...base, status: 'available', status_source: 'unreported', presence: 'unreported', presence_observed_at: null };
  assert.deepEqual(availabilityRows(unreported, null, now)[0], { key: 'status', title: 'Available', context: 'No device reporting · agents ask as usual' });
});

test('a word this build does not know reads as itself on every axis', () => {
  const later = { ...base, status: 'busy', interruptions: 'family', deadline_clock: 'hold-weekend', deadline_clock_since: at(12, 13), presence: 'driving' };
  assert.deepEqual(titles(availabilityRows(later, null, now)), [
    'Status: busy | From your devices: driving, reported 3m ago',
    'Interruptions: family | A setting this version of the app does not describe',
    'Deadline clock: hold-weekend | A setting this version of the app does not describe · since 13:00',
    'No note | A note tells agents what matters to you now',
  ]);
  assert.equal(availabilityRows({ ...base, status_source: 'calendar', status_until: at(12, 18) }, null, now)[0].context, 'Source: calendar · until 18:00');
  assert.equal(availabilityRows({ ...base, interruptions: 'constructor', deadline_clock: 'toString' }, null, now)[1].title, 'Interruptions: constructor', 'a word never reads an object property');
});

test('overrides name their source, start and end', () => {
  const set = {
    ...base, status: 'away', status_source: 'override', status_until: at(12, 18),
    interruptions: 'urgent', interruptions_until: at(13, 9),
    deadline_clock: 'hold-routine', deadline_clock_since: at(12, 13), deadline_clock_until: at(12, 16),
  };
  const note = { person: 'person/ada', author: 'person/ada', revision: 'r', text: 'Only the release matters today', time: at(12, 9), expires_at: at(13, 9) };
  assert.deepEqual(titles(availabilityRows(set, note, now)), [
    'Away | Set by you · until 18:00',
    'Interruptions: urgent only | Only asks with priority critical reach you · until tomorrow 09:00',
    'Deadline clock: routine asks held | Urgent asks keep running; held time is added to the rest · since 13:00 · until 16:00',
    'Only the release matters today | Note for agents · until tomorrow 09:00',
  ]);
  const open = availabilityRows({ ...set, status_until: null, deadline_clock: 'hold-all', deadline_clock_until: null }, { ...note, expires_at: null }, now);
  assert.equal(open[0].context, 'Set by you');
  assert.equal(open[2].context, 'Held time is added to every deadline · since 13:00');
  assert.equal(open[3].context, 'Note for agents · until you clear it');
});
