import { Agent, Arrangement, decodeUnknownSync } from '@smalltalk/st3-client/schema'
import type { Arrangement as ArrangementModel } from '@smalltalk/st3-client'
import { Schema } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import vectors from '../../../../fixtures/clients/sidebar-v2.json'
import { fixtureSource } from '../data/fixtureSource.ts'
import { arrangementSidebarDoc, type FolderState } from '../folders/client.ts'
import type { EditOutcome } from '../folders/edit.ts'

const vector = vectors.cases[1]!
const decoded = Schema.decodeUnknownSync(Schema.toEncoded(Arrangement))(vector.arrangement)
const arrangement: ArrangementModel = {
  id: decoded.id, kind: decoded.kind, owner: decoded.owner, body: decoded.body,
  deleted: decoded.deleted, revision: decoded.revision, updated_at: decoded.updated_at,
  ...(decoded.resolved == null ? {} : { resolved: decoded.resolved }),
}
export const nestingFolderSource = Atom.make<FolderState>({
  arrangement, doc: arrangementSidebarDoc(arrangement), phase: 'fixture', readOnly: true,
  edit: async (): Promise<EditOutcome> => ({ _tag: 'Refused', reason: { _tag: 'Unknown' }, detail: 'Read-only fixture.', targets: [] }),
})
export const nestingSource = fixtureSource({ world: {
  now: Date.parse('2026-10-15T00:00:00.000Z'),
  agents: vector.inventory.map((id) => decodeUnknownSync(Agent)({
    kind: 'agent', id, name: id.split('/').at(-1)!, host_id: 'host/fixture', runtime_ids: [],
    reachability: 'reachable', state: 'running', harness_state: 'idle', blocked_on: null,
    fault: null, revision: '1', updated_at: '2026-10-15T00:00:00.000Z',
  })),
  events: [], missions: [], attention: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
} })
