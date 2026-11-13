import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ClientError } from '../../clients/typescript/fractal-client/index.ts';
import { PresenceReporter } from './presence.ts';

const start = Date.UTC(2026, 10, 12, 14);
const iso = minutes => new Date(start + minutes * 60_000).toISOString();

// `answer` gives each report's `report_again_at` (in minutes from the start, or null) or throws.
function harness(answer = () => null) {
  const reports = [], timers = new Map();
  let clock = start, input = start, next = 0;
  const reporter = new PresenceReporter({
    report: async report => {
      reports.push(report);
      const again = answer(report);
      return { availability: { person: 'person/ada' }, report_again_at: again === null ? null : iso(again) };
    },
    lastInput: () => input,
    after: (run, ms) => { const handle = ++next; timers.set(handle, { run, at: clock + ms }); return () => timers.delete(handle); },
    now: () => clock,
  });
  return {
    reporter, reports, timers,
    /** Move the clock to `minutes`, running a timer due by then. */
    advance(minutes) {
      clock = start + minutes * 60_000;
      for (const [handle, { run, at }] of [...timers]) if (at <= clock) { timers.delete(handle); run(); }
    },
    touch(minutes) { clock = input = start + minutes * 60_000; reporter.input(); },
  };
}
const settle = () => new Promise(resolve => setImmediate(resolve));
const line = report => `${report.state} input ${report.last_input_at ?? '-'} at ${report.observed_at}`;

test('every change of state reports the raw state with the last input; a repeat of the same state does not', async () => {
  const { reporter, reports, advance } = harness();
  reporter.foreground(true);
  await settle();
  reporter.foreground(true);
  advance(3);
  reporter.foreground(false);
  await settle();
  advance(30);
  reporter.foreground(true);
  await settle();
  assert.deepEqual(reports.map(line), [
    `active input ${iso(0)} at ${iso(0)}`,
    `inactive input ${iso(0)} at ${iso(3)}`,
    `active input ${iso(0)} at ${iso(30)}`,
  ]);
});

test('at report_again_at, with nothing changed, it reports again with its newer last input', async () => {
  // Like the daemon: an active device turns idle 10 minutes after its last input; an inactive one names no time.
  const { reporter, reports, timers, advance, touch } = harness(report => report.state === 'active' ? (Date.parse(report.last_input_at) - start) / 60_000 + 10 : null);
  reporter.foreground(true);
  await settle();
  assert.equal(timers.size, 1);
  touch(4);
  advance(9);
  assert.equal(reports.length, 1, 'nothing before report_again_at');
  advance(10);
  await settle();
  assert.equal(line(reports[1]), `active input ${iso(4)} at ${iso(10)}`);
  touch(12);
  advance(14);
  await settle();
  assert.equal(line(reports[2]), `active input ${iso(12)} at ${iso(14)}`);
  reporter.foreground(false);
  await settle();
  assert.equal(line(reports[3]), `inactive input ${iso(12)} at ${iso(14)}`);
  advance(22);
  assert.equal(reports.length, 4, 'leaving cancels the pending report');
  assert.equal(timers.size, 0);
});

test('a null report_again_at waits for the first input, which reports once', async () => {
  const { reporter, reports, timers, advance, touch } = harness();
  reporter.foreground(true);
  await settle();
  advance(45);
  assert.equal(reports.length, 1);
  assert.equal(timers.size, 0);
  touch(46);
  touch(46.5);
  await settle();
  touch(47);
  await settle();
  assert.deepEqual(reports.slice(1).map(line), [`active input ${iso(46)} at ${iso(46)}`, `active input ${iso(47)} at ${iso(47)}`], 'each answer without a time waits for the next input');
});

test('a touch only records its time while a report_again_at is pending, and never reports while inactive', async () => {
  const { reporter, reports, touch, advance } = harness(report => report.state === 'active' ? 10 : null);
  reporter.foreground(true);
  await settle();
  touch(2);
  touch(3);
  assert.equal(reports.length, 1, 'a touch only records its time');
  reporter.foreground(false);
  await settle();
  touch(5);
  advance(60);
  await settle();
  assert.deepEqual(reports.map(report => report.state), ['active', 'inactive']);
});

test('nothing repeats while nothing changes: no heartbeat', async () => {
  const { reporter, reports, timers, advance } = harness();
  reporter.foreground(true);
  await settle();
  for (let minute = 10; minute <= 24 * 60; minute += 10) advance(minute);
  await settle();
  assert.equal(reports.length, 1);
  assert.equal(timers.size, 0);
});

test('a refusal stops reporting for the connection; another failure leaves the next input to report', async () => {
  const refused = new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/x', code: 'forbidden', message: 'the authenticated client session is not granted `write.presence`', retryable: false, details: {} }, 403);
  const denied = harness(() => { throw refused; });
  denied.reporter.foreground(true);
  await settle();
  denied.reporter.foreground(false);
  denied.touch(1);
  denied.reporter.foreground(true);
  assert.equal(denied.reports.length, 1);
  assert.equal(denied.timers.size, 0);

  let offline = true;
  const flaky = harness(() => { if (offline) throw new Error('Network request failed'); return 10; });
  flaky.reporter.foreground(true);
  await settle();
  offline = false;
  flaky.touch(2);
  await settle();
  assert.equal(flaky.reports.length, 2);
  assert.equal(flaky.timers.size, 1);
});

test('a stopped connection sends nothing more and its last answer schedules nothing', async () => {
  const { reporter, reports, timers, touch } = harness(() => 10);
  reporter.foreground(true);
  reporter.stop();
  await settle();
  reporter.foreground(false);
  touch(1);
  assert.equal(reports.length, 1);
  assert.equal(timers.size, 0);
});

test('an older answer never schedules over a newer report', async () => {
  const { reporter, timers, advance } = harness(report => report.state === 'active' ? 10 : null);
  reporter.foreground(true);
  advance(1);
  reporter.foreground(false);
  await settle();
  assert.equal(timers.size, 0, 'the active answer arrived after the inactive report was sent');
});
