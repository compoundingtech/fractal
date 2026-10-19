import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { SloCounters, SLO_BUCKET_UPPER_MS, SLO_COUNTERS, SLO_HISTOGRAMS, SLO_TARGETS, isQualityHour, judgeQuality, mergeQualityHour, qualityPercentile, retainQualityHours, sloBucket, sloHour } from '../slo.ts';
const empty = window => new SloCounters().line(window, 'host-test', 'build-test');

test('phone summary and targets match the Rust reference exactly', () => {
  const rust = JSON.parse(readFileSync(new URL('../../../../crates/stui/tests/fixtures/quality-hour.json', import.meta.url), 'utf8'));
  assert.deepEqual(empty(0), rust);
  assert.equal(isQualityHour(rust), true);
  assert.deepEqual(Object.keys(rust.counters).sort(), [...SLO_COUNTERS].sort());
  assert.deepEqual(Object.keys(rust.histograms).sort(), [...SLO_HISTOGRAMS].sort());
  assert.deepEqual(rust.targets, SLO_TARGETS);
});
test('inclusive buckets put fractional values above each boundary into the next bucket', () => {
  assert.equal(sloBucket(0), 0);
  for (let i = 0; i < 16; i++) {
    assert.equal(sloBucket(SLO_BUCKET_UPPER_MS[i]), i);
    assert.equal(sloBucket(SLO_BUCKET_UPPER_MS[i] + 0.000001), i + 1);
  }
  assert.equal(sloBucket(Infinity), 16);
  assert.equal(qualityPercentile([1, ...Array(15).fill(0), 19], 95), Infinity);
  assert.equal(qualityPercentile(Array(17).fill(0), 95), null);
});
test('fixed shape rejects content, unknown keys, fractional counts and invalid buckets', () => {
  const hour = empty(0);
  for (const extra of [{ text: 'private words' }, { person: 'person/ada' }, { path: '/home/example/draft' }]) assert.equal(isQualityHour({ ...hour, ...extra }), false);
  assert.equal(isQualityHour({ ...hour, counters: { ...hour.counters, 'agent/example': 1 } }), false);
  assert.equal(isQualityHour({ ...hour, counters: { ...hour.counters, launches: 1.5 } }), false);
  assert.equal(isQualityHour({ ...hour, bucket_upper_ms: [1] }), false);
  assert.equal(JSON.stringify(hour).includes('person'), false);
});
test('merging/rotation preserves later hours during a late partial shutdown and stays bounded', () => {
  let history = [];
  for (let i = 0; i < 400; i++) history = retainQualityHours(history, empty(i * 3600));
  assert.equal(history.length, 336);
  assert.equal(history[0].window_start, 64 * 3600);
  const add = new SloCounters(); add.add('launches');
  history = retainQualityHours(history, add.line(398 * 3600, 'host-test', 'new-build'));
  assert.equal(history.length, 336);
  assert.equal(history.at(-1).window_start, 399 * 3600);
  assert.equal(history.at(-2).counters.launches, 1);
  assert.equal(history.at(-2).build, 'new-build');
  history = retainQualityHours(history, empty(0));
  assert.equal(history.length, 336);
  const before = { ...empty(0), targets: { ...SLO_TARGETS, first_frame_p95_ms_lt: 200 } };
  assert.equal(mergeQualityHour(before, empty(0)).targets.first_frame_p95_ms_lt, 200);
});
test('judgments use historical targets, strict bounds, active-hour denominators and no invented successes', () => {
  const c = new SloCounters(); c.add('active_ms', 3_600_000); c.add('silent_exits'); c.add('conversation_opens', 100); c.add('open_cannot_load');
  c.add('direct_attach_attempts', 100); c.add('direct_attach_success', 99); c.observe('key_to_redraw_ms', 20);
  const hour = c.line(0, 'host-test', 'build-test'), rows = judgeQuality(hour);
  const row = target => rows.find(r => r.target === target);
  assert.equal(row('silent_exits_per_100_active_hours_lt').value, 100);
  assert.equal(row('conversation_error_percent_lt').pass, false);
  assert.equal(row('direct_attach_success_percent_gte').pass, true);
  assert.equal(row('key_to_redraw_p95_ms_lt').value, 32);
  assert.equal(row('key_to_redraw_p95_ms_lt').pass, true);
  assert.equal(row('terminal_open_p95_ms_lt').pass, null);
  const fatal = new SloCounters(); fatal.add('panics');
  assert.equal(judgeQuality(fatal.line(0, '', '')).find(r => r.target === 'panics_max').pass, false);
  hour.targets.key_to_redraw_p95_ms_lt = 32;
  assert.equal(judgeQuality(hour).find(r => r.target === 'key_to_redraw_p95_ms_lt').pass, false);
  c.add('launches', Number.MAX_SAFE_INTEGER); c.add('launches');
  assert.equal(c.line(0, '', '').counters.launches, Number.MAX_SAFE_INTEGER);
});
test('local hour includes half-hour timezone offsets', () => {
  const previous = process.env.TZ; process.env.TZ = 'Asia/Kolkata';
  try { assert.equal(sloHour(Date.parse('2026-01-01T12:44:12Z')), Date.parse('2026-01-01T12:30:00Z') / 1000); }
  finally { if (previous === undefined) delete process.env.TZ; else process.env.TZ = previous; }
});

test('repeated daylight-saving hours keep their own boundary instead of an already past hour', () => {
  const previous = process.env.TZ; process.env.TZ = 'America/New_York';
  try {
    for (const hour of ['2026-11-01T05', '2026-11-01T06', '2026-03-08T07']) {
      const at = Date.parse(`${hour}:30:12Z`);
      assert.equal(sloHour(at), Date.parse(`${hour}:00:00Z`) / 1000);
      assert.ok(sloHour(at) * 1000 + 3600_000 > at);
    }
  } finally { if (previous === undefined) delete process.env.TZ; else process.env.TZ = previous; }
});
