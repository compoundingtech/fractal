import { Agent, decodeUnknownSync } from '@smalltalk/st3-client/schema'
import * as Atom from 'effect/reactivity/Atom'
import vectors from '../../../../fixtures/clients/sidebar-v2.json'
import { fixtureSource } from '../data/fixtureSource.ts'
import { arrangementSidebarDoc, type FolderState } from '../folders/client.ts'
import { canonicalSidebarVector } from '../folders/sidebarV2.fixture.ts'

const vector = vectors.cases[1]!
export const { arrangement, folderId: nestingFolderId } = canonicalSidebarVector(vector)
export const nestingFolderSource = Atom.make<FolderState>({
  arrangement, doc: arrangementSidebarDoc(arrangement), phase: 'fixture', readOnly: true,
  edit: async () => ({ _tag: 'Refused', reason: { _tag: 'Unknown' }, detail: 'Read-only fixture.', targets: [] }),
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
