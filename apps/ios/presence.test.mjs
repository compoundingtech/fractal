import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ClientError } from '../../clients/typescript/fractal-client/index.ts';
import { PRESENT_EVERY_MS, PresenceReporter } from './presence.ts';

function harness(fail) {
  const reports = [], timers = new Map();
  let clock = Date.UTC(2026, 10, 12, 14), next = 0;
  const reporter = new PresenceReporter({
    report: async (presence, observedAt) => { reports.push(`${presence} ${observedAt}`); if (fail) throw fail; },
    after: (run, ms) => { const handle = ++next; timers.set(handle, { run, ms }); return () => timers.delete(handle); },
    now: () => clock,
  });
  const fire = () => { const [[handle, { run, ms }]] = timers; assert.equal(ms, PRESENT_EVERY_MS); timers.delete(handle); clock += ms; run(); };
  return { reporter, reports, timers, fire };
}
const settle = () => new Promise(resolve => setImmediate(resolve));

test('present while in front, repeated within the 15-minute freshness, idle once the app leaves', async () => {
  const { reporter, reports, timers, fire } = harness();
  reporter.foreground(true);
  assert.equal(timers.size, 1);
  fire();
  reporter.foreground(false);
  await settle();
  assert.deepEqual(reports, ['present 2026-11-12T14:00:00.000Z', 'present 2026-11-12T14:10:00.000Z', 'idle 2026-11-12T14:10:00.000Z']);
  assert.equal(timers.size, 0, 'nothing repeats in the background');
  assert.ok(PRESENT_EVERY_MS < 15 * 60_000 && PRESENT_EVERY_MS >= 5 * 60_000);
});

test('stopping a connection ends the repetition', () => {
  const { reporter, timers } = harness();
  reporter.foreground(true);
  reporter.stop();
  assert.equal(timers.size, 0);
});

test('a pairing refused write.presence stops reporting; other failures try again on the next report', async () => {
  const refused = new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/x', code: 'forbidden', message: 'the authenticated client session is not granted `write.presence`', retryable: false, details: {} }, 403);
  const denied = harness(refused);
  denied.reporter.foreground(true);
  await settle();
  assert.equal(denied.timers.size, 0);
  denied.reporter.foreground(true);
  assert.equal(denied.reports.length, 1);

  const offline = harness(new Error('Network request failed'));
  offline.reporter.foreground(true);
  await settle();
  assert.equal(offline.timers.size, 1);
});
