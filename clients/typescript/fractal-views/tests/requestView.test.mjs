import assert from 'node:assert/strict';
import { ANSWERS, askDeadline, isRequest, report, spaced, yesNo } from '@fractal/views/requestView';

// The same report the Fractal terminal UI's test reads (crates/fractal-tui/src/ui/screens.rs).
const text = 'The deploy finished: {"status":"failed","error":"unit st3.service did not start","commit":"8821eced","host":"willow","attempt":2,"log":"/var/log/x","duration_ms":1234,"members":["maple","cedar"],"plan":{"a":1},"notes":"long\\nnotes"}';
const shown = report(text);
assert.equal(shown.before, 'The deploy finished:');
assert.deepEqual(shown.rows.slice(0, 4).map(row => [row.key, row.value, row.tone]), [
  ['status', 'failed', 'fault'], ['error', 'unit st3.service did not start', 'fault'], ['commit', '8821eced', 'text'], ['host', 'willow', 'text'],
]);
assert.equal(shown.rows.length, 8, 'telling fields, then four others');
assert.equal(shown.more, 2);
assert.equal(shown.rows.find(row => row.key === 'notes'), undefined);
assert.equal(report('Can you look at the deploy?'), null);
assert.equal(report('{not json}'), null);

assert.ok(isRequest('person-step') && isRequest('agent-request') && !isRequest('review'));
assert.ok(yesNo('Should I merge it? ') && !yesNo('Please review the plan.'));
assert.equal(ANSWERS.nothing, 'Nothing for me to do here.');

// An agent's question gets room to read, as in the Fractal terminal UI: prose apart, a short label in bold.
assert.equal(
  spaced('Recommend: yes, in three parts.\nWhy: the release run failed.\nMy proposal:\n1. Land #1049.\n2. Build on main.\n```\ncargo build\nnext\n```\nAnswer yes and I queue it.'),
  '**Recommend:** yes, in three parts.\n\n**Why:** the release run failed.\n\nMy proposal:\n1. Land #1049.\n2. Build on main.\n```\ncargo build\nnext\n```\nAnswer yes and I queue it.',
);
assert.equal(spaced('The release run on the Linux runner failed: mold is missing'), 'The release run on the Linux runner failed: mold is missing');

// Words answer a structured request the way it takes them (Nathan, 2026-10-03: words on a custom
// choice were sent as a bare summary, refused, and the item stayed on Home).
{
  const { personAnswer } = await import('@fractal/views/requestView');
  const options = [{ id: 'works' }, { id: 'broken' }];
  assert.deepEqual(personAnswer({ type: 'choice', custom: true, answers: options }, undefined, ' it works '), { text: 'it works' });
  assert.deepEqual(personAnswer({ type: 'choice', custom: true, answers: options }, 'works', 'Works'), { id: 'works' });
  assert.deepEqual(personAnswer({ type: 'feedback' }, undefined, 'notes'), { text: 'notes' });
  assert.equal(personAnswer(undefined, undefined, 'notes'), undefined);
  const decision = { type: 'decision', answers: [{ id: 'land', outcome: 'accept' }, { id: 'hold', outcome: 'decline' }, { id: 'change', outcome: 'request_changes' }] };
  assert.deepEqual(personAnswer(decision, 'change', 'rename it'), { id: 'change', text: 'rename it' });
  assert.deepEqual(personAnswer(decision, undefined, 'rename it'), { id: 'change', text: 'rename it' });
  assert.equal(typeof personAnswer({ type: 'choice', answers: options }, undefined, 'words'), 'string');
}

// Every ask has a deadline; the card says how long is left and what then applies.
{
  const now = Date.parse('2026-10-26T08:00:00Z');
  assert.equal(
    askDeadline({ deadline_at: '2026-10-26T11:00:00Z', default_answer: { id: 'keep-open', label: 'Keep it open' } }, now),
    'answer within 3h, or “Keep it open” applies',
  );
  assert.equal(askDeadline({ deadline_at: '2026-10-28T08:00:00Z' }, now), 'answer within 2d, or it times out');
  assert.equal(askDeadline({}, now), null, 'an update or an older daemon has no deadline');
}
