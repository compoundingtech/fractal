import {
  ENTITY_DRAG_TYPE,
  entityDragPayload,
  entityKey,
  hoverCardTiming,
  parseEntityDragPayload,
  sameEntity,
  type EntityDescriptor,
} from './entity.ts'

// Standalone pure TypeScript suite: no DOM, React, runner or Node type dependency.
const assert = (condition: boolean, message: string): void => {
  if (!condition) throw new Error(message)
}
const equal = (actual: unknown, expected: unknown, message: string): void => {
  assert(JSON.stringify(actual) === JSON.stringify(expected), `${message}: ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`)
}
let tests = 0
const test = (name: string, body: () => void) => { body(); tests++; void name }

const agent: EntityDescriptor = { kind: 'agent', id: 'agent/alpha', title: 'Polish the sidebar', status: { tone: 'working', label: 'Working' } }

test('one key per kind and id, so a tab, a row and a mention resolve the same card', () => {
  equal(entityKey(agent), 'agent:agent/alpha', 'agent key')
  equal(entityKey({ kind: 'resource', id: 'github:pr/1' }), 'resource:github:pr/1', 'resource key keeps colons in the id')
  assert(sameEntity(agent, { kind: 'agent', id: 'agent/alpha' }), 'same kind and id is the same entity, whatever the title')
  assert(!sameEntity(agent, { kind: 'mission', id: 'agent/alpha' }), 'kind is part of identity')
})

test('the drag payload is the descriptor: kind, ref, title and status round-trip', () => {
  const payload = entityDragPayload(agent)
  equal(Object.keys(payload).sort(), [ENTITY_DRAG_TYPE, 'text/plain'].sort(), 'payload types')
  equal(payload['text/plain'], 'Polish the sidebar', 'plain text fallback is the title')
  equal(parseEntityDragPayload(payload[ENTITY_DRAG_TYPE]), agent, 'round trip')
})

test('a foreign or malformed payload is not an entity', () => {
  equal(parseEntityDragPayload(undefined), undefined, 'absent')
  equal(parseEntityDragPayload('not json'), undefined, 'not json')
  equal(parseEntityDragPayload(JSON.stringify({ kind: 'planet', id: 'x', title: 'x' })), undefined, 'unknown kind')
  equal(parseEntityDragPayload(JSON.stringify({ kind: 'agent', id: '', title: 'x' })), undefined, 'empty id')
  equal(parseEntityDragPayload(JSON.stringify({ kind: 'agent', id: 'a', title: 'x', status: { tone: 'loud', label: 'x' } })), { kind: 'agent', id: 'a', title: 'x' }, 'unknown status tone is dropped, not trusted')
})

test('the sidebar row timing is the canonical hover-card timing', () => {
  equal(hoverCardTiming, { openMs: 150, closeMs: 300 }, 'timing')
})

console.log(`entity: ${tests} tests passed`)
