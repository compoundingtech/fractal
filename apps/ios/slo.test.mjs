import assert from 'node:assert/strict';
import { bars, forecastOf, leftText, safeLink, secondLine, sloGroups, spanText } from './slo.ts';

// The same words the Fractal terminal UI uses (crates/fractal-tui/src/ui/objectives.rs).
assert.equal(leftText({ remaining_percent: 5.6 }), '6% left');
assert.equal(leftText({ remaining_percent: null }), 'no data');
assert.deepEqual([0, 59_000, 60_000, 3_600_000, 20 * 3_600_000, 86_400_000, 3 * 86_400_000].map(spanText), ['0s', '59s', '1m', '1h', '20h', '1d', '3d']);

const now = 1_000_000_000_000;
assert.deepEqual(forecastOf({ forecast_used_percent: 100, runs_out_at_unix_ms: now + 20 * 3_600_000 + 60_000 }, now),
  { short: 'out in 20h', long: 'runs out in 20h, before it resets', runsOut: true });
assert.deepEqual(forecastOf({ forecast_used_percent: 72.4 }, now),
  { short: '72% by reset', long: '72% used by the reset', runsOut: false });
assert.equal(forecastOf({}, now), null);

const budget = { id: 'anthropic-weekly', kind: 'budget', about: 'a', target: '100% per week', current: '94% used', window: 'weekly', remaining_percent: 6, status: 'met', explain: '', forecast_used_percent: 100, runs_out_at_unix_ms: now + 20 * 3_600_000 + 60_000 };
const objective = { ...budget, id: 'request-latency', kind: 'objective', current: 'p99 410ms', forecast_used_percent: undefined, runs_out_at_unix_ms: undefined };
assert.equal(secondLine(budget, now).text, '94% used · ');
assert.equal(secondLine(budget, now).forecast.short, 'out in 20h');
assert.equal(secondLine(objective, now).text, 'p99 410ms · target 100% per week · weekly');
assert.deepEqual(sloGroups([objective, budget]).map(group => [group.title, group.rows.length]), [['budgets', 1], ['objectives', 1]]);
assert.deepEqual(sloGroups([budget]).map(group => group.title), ['budgets']);

// A step with no reading is a dot, and a spent budget still shows a sliver.
assert.equal(bars({ steps: [100, 50, null, 0] }, 10), '█▄·▁');
assert.equal(bars({ steps: [100, 50, null, 0] }, 2), '·▁');

assert.equal(safeLink('https://otel.example.test/d/slo'), 'https://otel.example.test/d/slo');
assert.equal(safeLink('javascript:alert(1)'), null);
assert.equal(safeLink(undefined), null);
