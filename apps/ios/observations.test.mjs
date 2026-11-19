import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { KEEP_MS, MAX_INTERVALS_PER_REPORT, MAX_KEPT_INTERVALS, MAX_LATENCY_MS, MINUTE_MS, Observations, TARGETS, canonicalUpperMs, toObservationReport } from './observations.ts';

const T0 = Date.UTC(2026, 9, 10, 12, 0, 0); // a minute boundary
function make(options = {}) {
  const state = { now: T0 + 5_000, stored: options.stored ?? null, writes: 0, ids: 0, context: options.context ?? { carrier: 'fabric', path: 'direct' } };
  const storage = { read: async () => state.stored, write: async value => { state.stored = value; state.writes++; } };
  const clock = { now: () => state.now, id: () => `report-${++state.ids}` };
  const observations = new Observations(storage, clock, () => state.context);
  return { observations, state };
}
const settle = () => new Promise(resolve => setTimeout(resolve, 0));

// ---- Histogram: bins are named by the member's canonical inclusive upper bound, in whole ms.
// These pairs are what smallclaims::windows::histogram_upper_ms returns (pinned by a test there too).
for (const [ms, bound] of [[0, 0], [1, 1], [15, 15], [16, 16], [17, 17], [31, 31], [32, 33], [33, 33], [100, 103], [500, 511], [501, 511], [999, 1023], [1000, 1023], [1500, 1535], [3000, 3071], [8526, 8703], [10000, 10239], [59000, 59391], [3600000, 3670015]]) {
  assert.equal(canonicalUpperMs(ms), bound, `${ms}`);
}
assert.equal(canonicalUpperMs(MAX_LATENCY_MS + 5_000), canonicalUpperMs(MAX_LATENCY_MS), 'a latency past the member\'s limit is kept at it');
for (let ms = 0; ms < 70_000; ms += 7) {
  const bound = canonicalUpperMs(ms);
  assert.equal(canonicalUpperMs(bound), bound, `${ms} -> ${bound} is a bound of its own bin`);
  assert.ok(bound >= ms && bound - ms <= ms / 16, `${ms} -> ${bound}`);
}

/** The member's rules for a latency sample (crates/fractal-api-core/src/api/client_observations.rs), as far as they bear on what the app sends. */
function memberAccepts(sample) {
  const p99 = TARGETS[sample.target];
  const lower = bound => bound < 16 ? bound : bound + 1 - 2 ** (31 - Math.clz32(bound) - 4);
  let previous = -1, sum = 0, definitelyOver = 0, possiblyOver = 0;
  for (const [bound, count] of sample.buckets) {
    if (count <= 0 || bound > canonicalUpperMs(MAX_LATENCY_MS) || canonicalUpperMs(bound) !== bound || previous >= bound) return 'histogram requires sorted unique canonical inclusive ms bin bounds and positive counts';
    sum += count;
    if (lower(bound) > p99) definitelyOver += count;
    if (bound > p99) possiblyOver += count;
    previous = bound;
  }
  if (sum !== sample.count || sample.over_target < definitelyOver || sample.over_target > possiblyOver || sample.buckets.at(-1)[0] !== canonicalUpperMs(sample.max_ms)
    || (sample.max_ms <= p99 && sample.over_target !== 0) || (sample.max_ms > p99 && sample.over_target === 0)) return 'histogram, max and over-target count disagree';
  return null;
}

// ---- What the app sends is what the member accepts, whatever the latencies are.
{
  const { observations, state } = make();
  for (const target of Object.keys(TARGETS)) {
    for (const ms of [1, 16, 17, 58, 89, 300, TARGETS[target], TARGETS[target] + 1, 1000, 2500, 8526, 9999, 70_000, MAX_LATENCY_MS * 2]) observations.observe(target, ms);
  }
  state.now = T0 + 2 * MINUTE_MS;
  const report = toObservationReport((await observations.nextReport()));
  const latency = report.samples.filter(sample => sample.kind === 'latency');
  assert.equal(latency.length, Object.keys(TARGETS).length);
  for (const sample of latency) assert.equal(memberAccepts(sample), null, sample.target);
}

// ---- A latency lands in its minute, on its route, with its misses counted against its target.
{
  const { observations, state } = make();
  const token = observations.begin('ios-message-ack');
  state.now += 120;
  observations.end(token);
  observations.observe('ios-message-ack', 900);
  observations.observe('ios-open-to-live', 1400, { carrier: 'tailscale' });
  observations.observe('ios-recover', -5);
  observations.observe('ios-recover', Number.NaN);
  assert.equal((await observations.nextReport()), null, 'the minute in progress is not reported');
  state.now = T0 + MINUTE_MS + 1;
  const report = (await observations.nextReport());
  assert.equal(report.report_id, 'report-1');
  assert.equal(report.intervals.length, 1);
  const [interval] = report.intervals;
  assert.equal(interval.interval_start, '2026-10-10T12:00:00.000Z');
  assert.equal(interval.interval_end, '2026-10-10T12:01:00.000Z');
  const ack = interval.samples.find(sample => sample.target === 'ios-message-ack');
  assert.deepEqual({ ...ack, buckets: undefined }, { target: 'ios-message-ack', carrier: 'fabric', path: 'direct', count: 2, over_target: 1, max_ms: 900, buckets: undefined });
  assert.equal(ack.buckets.reduce((sum, [, count]) => sum + count, 0), 2);
  assert.ok(ack.buckets.every(([edge, count]) => Number.isInteger(edge) && count > 0));
  const open = interval.samples.find(sample => sample.target === 'ios-open-to-live');
  assert.deepEqual([open.carrier, open.path, open.count, open.over_target], ['tailscale', undefined, 1, 0]);
  assert.equal(interval.samples.length, 2, 'a negative or NaN latency is not a sample');
}

// ---- A closed interval is sent once, and a retry is the identical report, not a new one.
{
  const { observations, state } = make();
  observations.observe('ios-connect', 200);
  state.now = T0 + 2 * MINUTE_MS + 10;
  const first = (await observations.nextReport());
  // Samples after the batch was formed wait for the next one; the first is not rebuilt.
  observations.observe('ios-connect', 300);
  state.now += MINUTE_MS;
  const retry = (await observations.nextReport());
  assert.equal(JSON.stringify(retry), JSON.stringify(first));
  assert.equal(retry.report_id, 'report-1');
  (await observations.accepted('report-0'));
  assert.equal(JSON.stringify((await observations.nextReport())), JSON.stringify(first), 'another report ID accepts nothing');
  (await observations.accepted('report-1'));
  const next = (await observations.nextReport());
  assert.equal(next.report_id, 'report-2');
  assert.equal(next.intervals.length, 1);
  assert.equal(next.intervals[0].interval_start, '2026-10-10T12:02:00.000Z');
  (await observations.accepted('report-2'));
  assert.equal((await observations.nextReport()), null);
  assert.equal(observations.waiting, 0);
}

// ---- A report is bounded; the rest wait for the next. Old intervals are dropped; the keep is bounded.
{
  const { observations, state } = make();
  for (let minute = 0; minute < MAX_INTERVALS_PER_REPORT + 5; minute++) { state.now = T0 + minute * MINUTE_MS + 100; observations.observe('ios-connect', 100); }
  state.now += 2 * MINUTE_MS;
  const first = (await observations.nextReport());
  assert.equal(first.intervals.length, MAX_INTERVALS_PER_REPORT);
  (await observations.accepted(first.report_id));
  assert.equal((await observations.nextReport()).intervals.length, 5);
}
{
  const { observations, state } = make();
  observations.observe('ios-connect', 100);
  state.now = T0 + KEEP_MS + 2 * MINUTE_MS;
  assert.equal((await observations.nextReport()), null, 'seven days old: dropped, never sent late');
}
{
  const { observations, state } = make();
  for (let minute = 0; minute < MAX_KEPT_INTERVALS + 40; minute++) { state.now = T0 + minute * MINUTE_MS + 100; observations.observe('ios-connect', 100); }
  state.now += MINUTE_MS;
  (await observations.nextReport());
  assert.ok(observations.waiting <= MAX_KEPT_INTERVALS, 'the outbox is bounded while no member accepts');
}

// ---- The share of foreground time with a live feed: foreground is the denominator, live the part served.
{
  const { observations, state } = make();
  state.now = T0 + 50_000;
  observations.setForeground(true);
  state.now += 4_000;               // foreground, not live yet
  observations.setLive(true);
  state.now += 20_000;              // live across the minute boundary: 6 s in minute 0, 14 s in minute 1
  observations.setForeground(false);
  state.now += 30_000;              // background: counts for nothing
  observations.setForeground(true);
  state.now += 10_000;              // foreground and still live
  observations.setForeground(false);
  state.now = T0 + 2 * MINUTE_MS + 1;
  const report = (await observations.nextReport());
  const [m0, m1] = report.intervals;
  assert.deepEqual(m0.shares, [{ carrier: 'fabric', path: 'direct', foreground_ms: 10_000, live_ms: 6_000 }]);
  assert.equal(m1.shares[0].foreground_ms, 14_000 + 10_000);
  assert.equal(m1.shares[0].live_ms, 14_000 + 10_000);
  // The feed dropping while in the foreground is the miss.
}
{
  const { observations, state } = make();
  state.now = T0 + 1_000;
  observations.setForeground(true);
  observations.setLive(true);
  state.now += 10_000;
  observations.setLive(false);
  state.now += 5_000;
  observations.setForeground(false);
  state.now = T0 + MINUTE_MS + 1;
  const [interval] = (await observations.nextReport()).intervals;
  assert.deepEqual([interval.shares[0].foreground_ms, interval.shares[0].live_ms], [15_000, 10_000]);
}

// ---- What a report holds: no content, address, node ID or credential, only these keys.
{
  const { observations, state } = make({ context: { carrier: 'lan' } });
  observations.observe('ios-terminal-open', 1200);
  observations.setForeground(true);
  state.now += 1000;
  state.now = T0 + MINUTE_MS + 1;
  const report = (await observations.nextReport());
  assert.deepEqual(Object.keys(report), ['report_id', 'intervals']);
  assert.deepEqual(Object.keys(report.intervals[0]), ['interval_start', 'interval_end', 'samples', 'shares']);
  assert.deepEqual(Object.keys(report.intervals[0].samples[0]).sort(), ['buckets', 'carrier', 'count', 'max_ms', 'over_target', 'target']);
  assert.deepEqual(Object.keys(report.intervals[0].shares[0]).sort(), ['carrier', 'foreground_ms', 'live_ms']);
  assert.ok(Object.keys(TARGETS).every(name => name.startsWith('ios-')));
}

// ---- Kept across a restart: the batch with its ID, the closed intervals, and the minute in progress.
{
  const first = make();
  first.observations.observe('ios-connect', 200);
  first.state.now = T0 + MINUTE_MS + 10;
  first.observations.observe('ios-connect', 250);          // in minute 1
  const report = (await first.observations.nextReport());           // forms a batch of minute 0
  first.state.now += 10;
  await first.observations.flush();
  await settle();
  const second = make({ stored: first.state.stored });
  second.state.now = T0 + MINUTE_MS + 20;                   // the same minute as the one left open
  second.observations.observe('ios-connect', 300);
  await second.observations.load();
  assert.equal(JSON.stringify((await second.observations.nextReport())), JSON.stringify(report), 'the same batch, the same ID');
  (await second.observations.accepted(report.report_id));
  second.state.now = T0 + 3 * MINUTE_MS;
  const rest = (await second.observations.nextReport());
  assert.equal(rest.intervals.length, 1);
  const sample = rest.intervals[0].samples[0];
  assert.equal(sample.count, 2, 'the restored open minute and this run\'s sample are summed');
}
{
  const damaged = make({ stored: '{not json' });
  await damaged.observations.load();
  damaged.observations.observe('ios-connect', 100);
  damaged.state.now = T0 + MINUTE_MS + 1;
  assert.equal((await damaged.observations.nextReport()).intervals.length, 1, 'a damaged record is dropped and measuring goes on');
}

// ---- A minute that is closed twice is one interval: the member refuses a report with a minute twice.
{
  const { observations, state } = make();
  state.now = T0 + 10_000;
  observations.observe('ios-connect', 100);                 // minute 0
  state.now = T0 + 20_000;
  observations.setForeground(true);
  observations.setLive(true);
  state.now = T0 + MINUTE_MS + 10_000;
  observations.observe('ios-connect', 200);                 // minute 1 begins: minute 0 is closed, but the share is still in it
  state.now += 5_000;
  observations.setLive(false);                              // the rest of minute 0 is counted and it is closed again
  state.now = T0 + 3 * MINUTE_MS;
  const report = (await observations.nextReport());
  const starts = report.intervals.map(interval => interval.interval_start);
  assert.deepEqual(starts, [...new Set(starts)], 'no minute twice');
  const first = report.intervals[0];
  assert.equal(first.samples.length, 1);
  assert.equal(first.shares.length, 1);
  assert.equal(first.shares[0].foreground_ms, 40_000, 'both closes of minute 0 are summed');
}

// ---- A report the member refuses for its content is dropped; one it never saw is kept.
{
  const { observations, state } = make();
  observations.observe('ios-connect', 100);
  state.now = T0 + 2 * MINUTE_MS;
  const first = (await observations.nextReport());
  assert.equal(observations.waiting, 1);
  (await observations.refused('some-other-report'));
  assert.equal((await observations.nextReport()).report_id, first.report_id, 'a refusal of another report changes nothing');
  (await observations.refused(first.report_id));
  assert.equal(observations.waiting, 0);
  assert.equal((await observations.nextReport()), null, 'the refused batch is not sent again');
  observations.observe('ios-connect', 100);
  state.now = T0 + 4 * MINUTE_MS;
  assert.notEqual((await observations.nextReport()).report_id, first.report_id, 'later measurements are reported');
}

// ---- What an earlier build kept named its bins by index and the member never took it: it is not sent.
{
  const { observations, state } = make({ stored: JSON.stringify({ closed: [{ start: T0, series: { 'x|fabric|direct': { target: 'ios-connect', carrier: 'fabric', path: 'direct', count: 1, over_target: 0, max_ms: 100, buckets: { 106: 1 } } }, shares: {} }] }) });
  await observations.load();
  assert.equal(observations.waiting, 0);
  state.now = T0 + 2 * MINUTE_MS;
  assert.equal((await observations.nextReport()), null);
}

// ---- Before a route is chosen nothing is reported as a carrier.
{
  const { observations, state } = make({ context: { carrier: 'none' } });
  observations.observe('ios-open-to-live', 900);
  state.now = T0 + 2 * MINUTE_MS;
  assert.equal(toObservationReport((await observations.nextReport())).samples.length, 0);
}

// ---- A timer that is cancelled, or never ended, records nothing and does not pile up.
{
  const { observations, state } = make();
  const token = observations.begin('ios-recover');
  observations.cancel(token);
  state.now += 500;
  observations.end(token);
  for (let i = 0; i < 100; i++) observations.begin('ios-recover');
  state.now = T0 + MINUTE_MS + 1;
  assert.equal((await observations.nextReport()), null);
}

// ---- A report is what the member takes: flat samples (at most 32), each naming its minute; an unknown carrier is left out.
{
  const { observations, state } = make({ context: { carrier: 'tailscale' } });
  const token = observations.begin('ios-message-ack');
  state.now += 120;
  observations.end(token);
  observations.observe('ios-open-to-live', 900, { carrier: 'https' });
  observations.setForeground(true);
  observations.setLive(true);
  state.now += 20_000;
  observations.setLive(false);
  state.now = T0 + 2 * MINUTE_MS + 100;
  const batch = (await observations.nextReport());
  const report = toObservationReport(batch);
  assert.equal(report.report_id, batch.report_id);
  assert.ok(report.samples.length >= 1 && report.samples.length <= 32);
  const latency = report.samples.find(sample => sample.kind === 'latency');
  assert.equal(latency.target, 'ios-message-ack');
  assert.equal(latency.carrier, 'tailscale');
  assert.equal(latency.interval_start, new Date(T0).toISOString());
  assert.equal(latency.count, 1);
  assert.ok(!report.samples.some(sample => sample.carrier === 'https'), 'a carrier the member does not know is not reported');
  for (const sample of report.samples.filter(sample => sample.kind === 'live-share')) {
    assert.equal(sample.target, 'ios-live-share');
    assert.ok(sample.foreground_ms >= 1 && sample.foreground_ms <= 60_000 && sample.live_ms <= sample.foreground_ms);
  }
}
{
  // Many targets in one minute never make a report over the member's 32 samples.
  const { observations, state } = make();
  for (let minute = 0; minute < 12; minute++) {
    state.now = T0 + minute * MINUTE_MS + 100;
    for (const target of Object.keys(TARGETS)) for (const carrier of ['fabric', 'tailscale', 'lan']) observations.observe(target, 100, { carrier });
  }
  state.now += 2 * MINUTE_MS;
  const report = toObservationReport((await observations.nextReport()));
  assert.ok(report.samples.length <= 32, `${report.samples.length}`);
}

// ---- Real disk + process restart boundary: the native test consumes exactly this producer output.
{
  const directory = await mkdtemp(join(tmpdir(), 'ios-observation-minute-'));
  try {
    const path = join(directory, 'observations.json');
    const storage = {
      read: async () => { try { return await readFile(path, 'utf8'); } catch (error) { if (error.code === 'ENOENT') return null; throw error; } },
      write: value => writeFile(path, value, 'utf8'),
    };
    const minute = Date.UTC(2026, 10, 19, 12, 0);
    let now = minute;
    const clock = { now: () => now, id: () => '11111111-1111-4111-8111-111111111111' };
    const context = () => ({ carrier: 'fabric', path: 'direct' });
    const producer = new Observations(storage, clock, context);
    await producer.load();
    for (const target of Object.keys(TARGETS)) producer.observe(target, 100);
    producer.setForeground(true);
    producer.setLive(true);
    now += 20_000;
    producer.setLive(false);
    now = minute + MINUTE_MS;
    producer.setForeground(false);
    const batch = await producer.nextReport();
    const fixture = JSON.parse(await readFile(new URL('./fixtures/observation-minute.json', import.meta.url), 'utf8'));
    assert.deepEqual(toObservationReport(batch), fixture);
    // No flush is needed: upload permission itself proves the report has been saved.
    const saved = JSON.parse(await storage.read());
    assert.equal(saved.batch.report_id, fixture.report_id);
    const restarted = new Observations(storage, { now: () => now, id: () => { throw new Error('retry must not allocate a new ID'); } }, context);
    assert.deepEqual(toObservationReport(await restarted.nextReport()), fixture);
    await restarted.accepted(batch.report_id);
    now += 2 * MINUTE_MS;
    const afterAcceptance = new Observations(storage, clock, context);
    assert.equal(await afterAcceptance.nextReport(), null, 'unobserved minutes stay absent, not zero-valued claims');
  } finally { await rm(directory, { recursive: true, force: true }); }
}

// ---- Failed storage is neither permission to upload nor acknowledgement of unsaved removal.
{
  let now = T0 + 100;
  let stored = null;
  let failRead = false;
  let failWrite = true;
  let ids = 0;
  const storage = {
    read: async () => { if (failRead) throw new Error('read unavailable'); return stored; },
    write: async value => { if (failWrite) throw new Error('disk unavailable'); stored = value; },
  };
  const clock = { now: () => now, id: () => `durable-${++ids}` };
  const context = () => ({ carrier: 'tailscale' });
  const producer = new Observations(storage, clock, context);
  producer.observe('ios-connect', 100);
  now += MINUTE_MS;
  await assert.rejects(producer.nextReport(), /disk unavailable/);
  assert.equal(producer.waiting, 1);
  failWrite = false;
  const batch = await producer.nextReport();
  assert.equal(batch.report_id, 'durable-1', 'failed save retains the same pending ID');
  failWrite = true;
  await assert.rejects(producer.accepted(batch.report_id), /disk unavailable/);
  assert.equal(producer.waiting, 1, 'failed acknowledgement save retains the batch');
  await assert.rejects(producer.refused(batch.report_id), /disk unavailable/);
  failWrite = false;
  assert.deepEqual(await producer.nextReport(), batch);
  failRead = true;
  const restarted = new Observations(storage, clock, context);
  await assert.rejects(restarted.nextReport(), /read unavailable/);
  failRead = false;
  assert.deepEqual(await restarted.nextReport(), batch, 'a failed read never replaces the saved authority');
}

// ---- A report cannot leave preparation while its journal write is still pending.
{
  const saving = Promise.withResolvers();
  const started = Promise.withResolvers();
  let now = T0 + 100;
  let stored = null;
  const storage = {
    read: async () => stored,
    write: async value => {
      if (JSON.parse(value).batch) { started.resolve(); await saving.promise; }
      stored = value;
    },
  };
  const producer = new Observations(storage, { now: () => now, id: () => 'pending-report' }, () => ({ carrier: 'lan' }));
  producer.observe('ios-connect', 100);
  now += MINUTE_MS;
  let uploadable = false;
  const preparing = producer.nextReport().then(batch => { uploadable = true; return batch; });
  await started.promise;
  assert.equal(uploadable, false, 'pending disk write must not permit delivery');
  saving.resolve();
  const batch = await preparing;
  assert.equal(JSON.parse(stored).batch.report_id, batch.report_id);
  assert.deepEqual(await producer.nextReport(), batch, 'retry retains the exact saved payload');
}

// ---- Background saves cannot resurrect a batch while its durable removal is pending.
for (const trigger of ['flush', 'sweep']) {
  const removing = Promise.withResolvers();
  const release = Promise.withResolvers();
  let now = T0 + 100;
  let stored = null;
  let blockRemoval = false;
  let ids = 0;
  const storage = {
    read: async () => stored,
    write: async value => {
      if (blockRemoval && !JSON.parse(value).batch) {
        removing.resolve();
        await release.promise;
      }
      stored = value;
    },
  };
  const clock = { now: () => now, id: () => `serialized-${++ids}` };
  const context = () => ({ carrier: 'lan' });
  const producer = new Observations(storage, clock, context);
  producer.observe('ios-connect', 100);
  now += MINUTE_MS;
  const batch = await producer.nextReport();
  await producer.flush();
  blockRemoval = true;
  const accepting = producer.accepted(batch.report_id);
  await removing.promise;
  if (trigger === 'sweep') {
    producer.observe('ios-recover', 200);
    now += MINUTE_MS;
    producer.observe('ios-recover', 300); // closes a minute, scheduling an automatic save
  }
  const backgrounded = producer.flush();
  release.resolve();
  await accepting;
  await backgrounded;
  assert.equal(JSON.parse(stored).batch, undefined, `${trigger} must not save the acknowledged batch again`);
  const restarted = new Observations(storage, clock, context);
  const next = await restarted.nextReport();
  if (trigger === 'flush') assert.equal(next, null, 'restart has no acknowledged interval');
  else {
    assert.notEqual(next.report_id, batch.report_id);
    assert.deepEqual(next.intervals.map(interval => interval.interval_start), [new Date(T0 + MINUTE_MS).toISOString()]);
  }
}
