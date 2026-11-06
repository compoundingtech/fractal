import * as Native from '@fractal/client/schema'
import type { AttentionEncoded } from '@fractal/client/schema/read'
import { Option } from 'effect'
import { describe, expect, it } from 'vitest'
import { projectNativeDeclarations } from '../data/nativeDeclarations.ts'
import { observed, unavailable, waiting } from '../data/source.ts'
import { declaredEffort, isOpenPersonAsk, openPersonAskCount } from './agentMetadata.ts'

const agentRef = 'agent/example-seat'
const ask = {
  kind: 'attention', id: 'attention/example-ask', source_id: 'step-run/example/ask', requester_id: agentRef,
  conversation_id: agentRef, conversation_ids: [agentRef], person_id: 'person/operator', attention_kind: 'person-step',
  title: 'Choose the rollout', detail: 'Example ask', priority: 'normal', state: 'open', actions: ['work.done'],
  revision: '1', requested_at: '2026-10-15T12:00:00Z', updated_at: '2026-10-15T12:00:00Z',
} satisfies AttentionEncoded
const card = (fields: Partial<AttentionEncoded>) => Native.decodeUnknownSync(Native.Attention)({ ...ask, ...fields })
const declaration = (harness: readonly unknown[]) => Native.decodeUnknownSync(Native.AgentDeclaration)({
  id: agentRef, revision: '1', revisions: ['1'], kdl: 'never emitted',
  tree: { name: 'agent', children: [{ name: 'harness', arguments: ['omp'], children: harness }] },
})

describe('open person asks', () => {
  it('counts only open, non-update person asks shown in this conversation', () => {
    expect(isOpenPersonAsk({ item: card({}), agentRef })).toBe(true)
    for (const fields of [
      { state: 'resolved' },
      { update: { type: 'update', version: 1, about: 'mission/example', summary: 'Example update' } },
      { attention_kind: 'human-gate', actions: ['review.approve'] },
      { attention_kind: 'harness-prompt', actions: ['prompt.respond'] },
      { attention_kind: 'launch-approval', actions: ['launch.approve'] },
      { attention_kind: 'unread-message', actions: ['message.read'] },
      { conversation_id: 'agent/example-other', conversation_ids: ['agent/example-other'] },
    ] satisfies Partial<AttentionEncoded>[]) expect(isOpenPersonAsk({ item: card(fields), agentRef }), JSON.stringify(fields)).toBe(false)
  })
  it('names the asking agent from the requester on daemons that predate conversations', () => {
    const { conversation_id: _id, conversation_ids: _ids, ...older } = ask
    expect(isOpenPersonAsk({ item: Native.decodeUnknownSync(Native.Attention)(older), agentRef })).toBe(true)
  })
  it('is exact over complete attention and none otherwise, never a lower bound', () => {
    const items = [card({ id: 'attention/a' }), card({ id: 'attention/b' }), card({ id: 'attention/c', state: 'resolved' })]
    expect(openPersonAskCount({ feed: { ...observed({ value: items }), coverage: { _tag: 'Complete' } }, agentRef })).toEqual(Option.some(2))
    expect(openPersonAskCount({ feed: { ...observed({ value: [] }), coverage: { _tag: 'Complete' } }, agentRef })).toEqual(Option.some(0))
    expect(openPersonAskCount({ feed: { ...observed({ value: items, freshness: 'stale' }), coverage: { _tag: 'Complete' } }, agentRef })).toEqual(Option.some(2))
    expect(openPersonAskCount({ feed: observed({ value: items }), agentRef })).toEqual(Option.none())
    expect(openPersonAskCount({ feed: { ...observed({ value: items }), coverage: { _tag: 'Partial' } }, agentRef })).toEqual(Option.none())
    expect(openPersonAskCount({ feed: waiting, agentRef })).toEqual(Option.none())
    expect(openPersonAskCount({ feed: unavailable({ reason: 'ungranted', detail: 'Example refusal' }), agentRef })).toEqual(Option.none())
  })
})

describe('declared effort', () => {
  it('reads the harness effort child beside the declared model', () => {
    const projected = projectNativeDeclarations({ declaration: declaration([{ name: 'model', arguments: ['declared-model'] }, { name: 'effort', arguments: ['high'] }]) })
    expect(projected).toEqual({ agentRef, revision: '1', bindings: [], declaredModel: 'declared-model', declaredEffort: 'high' })
    expect(declaredEffort(observed({ value: { agentRef, revision: '1', bindings: [], declaredEffort: 'high' } }))).toEqual(Option.some('high'))
  })
  it('omits undeclared, malformed and misplaced effort rather than inventing one', () => {
    for (const harness of [
      [],
      [{ name: 'effort' }],
      [{ name: 'effort', arguments: [3] }],
      [{ name: 'effort', arguments: ['high effort; rm'] }],
      [{ name: 'model', arguments: ['declared-model'], children: [{ name: 'effort', arguments: ['high'] }] }],
    ]) {
      const projected = projectNativeDeclarations({ declaration: declaration(harness) })
      expect(projected, JSON.stringify(harness)).toBeDefined()
      expect(projected?.declaredEffort, JSON.stringify(harness)).toBeUndefined()
    }
    expect(declaredEffort(observed({ value: { agentRef, revision: '1', bindings: [] } }))).toEqual(Option.none())
    expect(declaredEffort(waiting)).toEqual(Option.none())
    expect(declaredEffort(unavailable({ reason: 'unsupported', detail: 'Example refusal' }))).toEqual(Option.none())
  })
})
