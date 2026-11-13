import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ClientError } from '../../clients/typescript/fractal-client/index.ts';
import { AXES, CLEARED, CUSTOM_HINT, REPAIR_FOR_AVAILABILITY, UNTIL_PRESETS, availabilityRows, axisChange, customUntil, ownAvailability, presetUntil, servesAvailability, whenText, writeRefusal } from './availability.ts';

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

test('the paired session reads the concrete person resolved by the availability API', () => {
  const capabilities = { session_actor: 'person/ada/session/phone' };
  const note = { person: 'person/ada', author: 'person/ada', revision: 'r', text: 'ship it', time: at(12, 9) };
  assert.notEqual(capabilities.session_actor, base.person);
  const own = ownAvailability([base], [{ ...note, person: 'person/avery' }, note]);
  assert.deepEqual(own, { person: base, note });
  assert.equal(availabilityRows(own.person, own.note, now)[3].title, 'ship it');
  assert.deepEqual(ownAvailability([base], []), { person: base, note: null });
  assert.deepEqual(ownAvailability([], [note]), { person: null, note: null });
  assert.deepEqual(ownAvailability([base, { ...base, person: 'person/avery' }], [note]), { person: null, note: null });
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

test('presets end in 30 minutes, one or two hours, or at 09:00 tomorrow even before 09:00 today', () => {
  assert.deepEqual(UNTIL_PRESETS.map(preset => presetUntil(preset, now)), [
    new Date(2026, 10, 12, 14, 30), new Date(2026, 10, 12, 15), new Date(2026, 10, 12, 16), new Date(2026, 10, 13, 9),
  ]);
  assert.deepEqual(presetUntil('Until tomorrow 9:00', new Date(2026, 10, 12, 7, 30)), new Date(2026, 10, 13, 9));
  assert.deepEqual(presetUntil('Until tomorrow 9:00', new Date(2026, 11, 31, 23)), new Date(2027, 0, 1, 9));
});

test('a typed end is the next such clock time or a span from now', () => {
  assert.deepEqual(customUntil('17:30', now), new Date(2026, 10, 12, 17, 30));
  assert.deepEqual(customUntil(' 8.15 ', now), new Date(2026, 10, 13, 8, 15));
  assert.deepEqual(customUntil('14:00', now), new Date(2026, 10, 13, 14), 'now itself has passed');
  assert.deepEqual(customUntil('45m', now), new Date(2026, 10, 12, 14, 45));
  assert.deepEqual(customUntil('1h30m', now), new Date(2026, 10, 12, 15, 30));
  assert.deepEqual(customUntil('3 h', now), new Date(2026, 10, 12, 17));
  for (const typed of ['', '0m', '24:00', '9:60', 'soon', '5']) assert.equal(customUntil(typed, now), CUSTOM_HINT, typed);
});

test('a default value clears its axis without an end; any other carries its end', () => {
  for (const axis of Object.keys(AXES)) assert.ok(AXES[axis].choices.some(([, value]) => value === CLEARED[axis]), axis);
  assert.deepEqual(axisChange('status', null, null), { status: { value: null } });
  assert.deepEqual(axisChange('interruptions', 'urgent', new Date(Date.UTC(2026, 10, 12, 18))), { interruptions: { value: 'urgent', until: '2026-11-12T18:00:00.000Z' } });
  assert.deepEqual(axisChange('deadline_clock', 'hold-all', null), { deadline_clock: { value: 'hold-all' } });
});

test('a pairing without the write scopes is told to re-pair; an agent is told only a person writes', () => {
  const refused = (status, code, message) => new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/x', code, message, retryable: false, details: {} }, status);
  assert.equal(writeRefusal(refused(403, 'forbidden', 'the authenticated client session is not granted `write.availability`')), REPAIR_FOR_AVAILABILITY);
  assert.match(writeRefusal(refused(403, 'forbidden', 'only a person writes their own availability, presence or note; agents are refused')), /^Only a person changes/);
  assert.doesNotMatch(writeRefusal(refused(422, 'validation-failed', 'status.until must be in the future')), /re-pair/i);
});
