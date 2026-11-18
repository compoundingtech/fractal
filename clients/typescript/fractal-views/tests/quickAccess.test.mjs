import assert from 'node:assert/strict';
import test from 'node:test';
import { quickAccessDestination, quickAccessItems, quickAccessSelection, planQuickAccessPlace } from '../quickAccess.ts';

const selection = quickAccessSelection('person/example/session/phone');
const row = (member, key, bucket = null) => ({ id: member, member, container: selection.subject, position: { key, bucket }, revision: 'claim/test' });
const members = [row('agent/example', 'a0'), row('mission/example', 'a1'), row('doc/plan', 'a2')];

test('one reserved collection belongs to the discovered person across sessions', () => {
  assert.deepEqual(selection, quickAccessSelection('person/example/session/web'));
  assert.deepEqual(selection, { owner: 'person/example', subject: 'arrangement/person/example/00000000-0000-7000-8000-000000000002' });
  for (const actor of ['agent/example', 'person/', 'person/a/session/', 'person/a/b', 'person/a\n']) assert.throws(() => quickAccessSelection(actor));
});

test('stable navigation identities do not capture an agent session or inbox item', () => {
  for (const [subject, kind] of [['agent/test/seat', 'agent'], ['mission/plan', 'mission'], ['doc/plan', 'document'], ['glass/example', 'glass'], ['resource/example', 'resource']])
    assert.deepEqual(quickAccessDestination(subject), { subject, kind });
  for (const subject of ['session/a', 'pty/a', 'attention/a', 'doc/plan@' + 'a'.repeat(64), 'agent/', 'agent//a', 'agent/a b', 'agent/' + 'ä'.repeat(254)])
    assert.equal(quickAccessDestination(subject), null, subject);
});

test('local unavailability preserves authoritative membership and order', () => {
  const shown = quickAccessItems([...members, row('external/not-supported-here', 'a3')], destination => destination.kind === 'agent' ? { title: 'Current agent', detail: 'Session changed' } : undefined);
  assert.deepEqual(shown.map(item => item.id), [...members.map(item => item.member), 'external/not-supported-here']);
  assert.equal(shown[0].destination.subject, 'agent/example');
  assert.equal(shown[0].title, 'Current agent');
  assert.equal(shown[1].availability, 'unavailable');
  assert.equal(shown[3].availability, 'unsupported');
  assert.equal(members.length, 3);
});

test('reorder changes only the member position and refuses a lost anchor', () => {
  const plan = planQuickAccessPlace(members, 'doc/plan', 'mission/example', (a, b) => { assert.deepEqual([a, b], ['a0', 'a1']); return 'a0V'; });
  assert.deepEqual(plan, { operations: [{ op: 'place', member: 'doc/plan', bucket: null, key: 'a0V' }], rebalanced: false });
  assert.deepEqual(planQuickAccessPlace(members, 'agent/new', 'mission/gone', () => 'a3'), { refusal: 'anchor-missing' });
  assert.deepEqual(planQuickAccessPlace(members, 'session/new', null, () => 'a3'), { refusal: 'unsupported-destination' });
  assert.deepEqual(planQuickAccessPlace(members, 'doc/plan', null, () => { throw Error('unused'); }), { operations: [], rebalanced: false });
});

test('concurrent equal keys rebalance the observed order as one bounded edit', () => {
  const collided = [row('agent/example', 'a0'), row('mission/example', 'a0')];
  const plan = planQuickAccessPlace(collided, 'doc/plan', 'mission/example', () => { throw Error('no gap'); });
  assert.equal(plan.rebalanced, true);
  assert.deepEqual(plan.operations.map(item => item.member), ['agent/example', 'doc/plan', 'mission/example']);
  assert.deepEqual(plan.operations.map(item => item.key), ['c000', 'c001', 'c002']);
});

test('invalid collection or generator result never emits a membership write', () => {
  assert.deepEqual(planQuickAccessPlace([row('agent/example', 'a0', 'folder')], 'doc/plan', null, () => 'a1'), { refusal: 'invalid-memberships' });
  assert.deepEqual(planQuickAccessPlace(members, 'agent/new', null, () => 'a0'), { refusal: 'ordering-unavailable' });
  assert.deepEqual(planQuickAccessPlace(members, 'agent/new', null, () => { throw Error('generator failed'); }), { refusal: 'ordering-unavailable' });
  assert.deepEqual(planQuickAccessPlace([...members, members[0]], 'agent/new', null, () => 'a3'), { refusal: 'invalid-memberships' });
});


test('the edit-operation limit does not cap the shared collection size', () => {
  const many = Array.from({ length: 1100 }, (_, index) => row(`agent/test-${index}`, `c${index.toString(36).padStart(3, '0')}`));
  assert.deepEqual(planQuickAccessPlace(many, 'mission/new', null, () => 'czzz'), {
    operations: [{ op: 'place', member: 'mission/new', bucket: null, key: 'czzz' }], rebalanced: false,
  });
  assert.deepEqual(planQuickAccessPlace(many, 'mission/new', null, () => null), { refusal: 'too-many-members' });
});
