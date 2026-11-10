import { describe, expect, it } from 'vitest'
import { Schema } from 'effect'
import * as Wire from '@fractal/client/schema'
import vectors from '../../../../fixtures/clients/client-control-lifecycle-v1.json'
import { testCommand, testGrant } from './command.test-support.ts'
import { createClientControlLifecycle } from './lifecycle.ts'

const decodeLifecycle = Wire.decodeUnknownSync(Wire.ClientControlGrantLifecycle, 'strict')
const decodeCommand = Wire.decodeUnknownSync(Wire.DeviceCommand, 'strict')
const encodeGrant = Schema.encodeSync(Wire.ClientControlGrant)
const [started, ended, revoked] = vectors.resources
if (started === undefined || ended === undefined || revoked === undefined) throw new Error('Missing canonical lifecycle resources')
const snapshot = vectors.frames.find(frame => frame.kind === 'snapshot')
const changes = vectors.frames.find(frame => frame.kind === 'changes')
if (snapshot === undefined || changes === undefined) throw new Error('Missing canonical lifecycle frames')

const grant: Wire.ClientControlGrant = {
  ...testGrant(), grant_id: started.grant_id, item_id: started.item_id,
  run_id: started.run_id, device: started.device, session: started.session,
}
const command = (binding: Wire.ClientControlGrant = grant): Wire.DeviceCommand => decodeCommand({
  ...testCommand('phone.inspect'), grant_id: binding.grant_id, run_id: binding.run_id,
  device_id: binding.device, session_actor: binding.session, control_grant: encodeGrant(binding),
})
const makeGate = (authority: (grantId: string) => Wire.ClientControlGrant | undefined = id => id === grant.grant_id ? grant : undefined) =>
  createClientControlLifecycle({ authority, subscriptionId: snapshot.id })
const refusedFrame = { accepted: false, decisions: [] }

// These fixtures are decoder/acceptance vectors, not enrollment authority. Only this
// explicit test callback supplies verified authority; the application remains disabled.
describe('client-control lifecycle gate', () => {
  it('loads every canonical resource and frame through the generated strict schemas', () => {
    const resourcesGate = makeGate()
    const framesGate = makeGate()
    const resourceDecisions = ['activate', 'terminate', 'remain-terminal']
    for (const [index, resource] of vectors.resources.entries()) {
      expect(() => decodeLifecycle(resource)).not.toThrow()
      expect(resourcesGate.acceptResource(resource)).toBe(resourceDecisions[index])
    }
    for (const [index, frame] of vectors.frames.entries()) {
      expect(() => Wire.decodeUnknownSync(Wire.CollectionFrame, 'strict')(frame)).not.toThrow()
      expect(framesGate.acceptFrame(frame)).toEqual({ accepted: true, decisions: [resourceDecisions[index]] })
    }
    expect(resourcesGate.enrollmentRevoked()).toBe(true)
    expect(framesGate.enrollmentRevoked()).toBe(true)
  })

  it('replays every shared trace, including schema-invalid negative resources', () => {
    for (const trace of vectors.traces) {
      const gate = makeGate()
      expect(trace.grant_id).toBe(grant.grant_id)
      expect(trace.item_id).toBe(grant.item_id)
      for (const step of trace.steps) {
        if ('resource' in step && step.resource !== undefined) {
          const resource = step.resource
          if (step.expected === 'refuse-invalid-frame') {
            expect(() => decodeLifecycle(resource)).toThrow()
            expect(gate.acceptResource(resource)).toBe('refused-invalid')
          } else {
            expect(gate.acceptResource(resource)).toBe(step.expected)
          }
        } else if ('command_grant_id' in step) {
          expect(step.command_grant_id).toBe(grant.grant_id)
          expect(gate.allows(command())).toBe(step.expected === 'allow-lifecycle-gate-only')
        } else {
          throw new Error('Unknown lifecycle trace step')
        }
      }
    }
  })

  it('requires separately verified authority and an embedded command grant', () => {
    const gate = makeGate(() => undefined)
    expect(gate.acceptResource(started)).toBe('refused-binding')
    expect(gate.acceptResource(revoked)).toBe('refused-binding')
    expect(gate.activeRunId(grant.grant_id)).toBeUndefined()
    expect(gate.enrollmentRevoked()).toBe(false)
    expect(gate.allows(command())).toBe(false)
    const verified = makeGate()
    expect(verified.allows(command())).toBe(false)
    expect(verified.acceptResource(started)).toBe('activate')
    const { control_grant: _control, ...withoutGrant } = command()
    expect(verified.allows(withoutGrant)).toBe(false)
    expect(verified.allows(command())).toBe(true)
  })

  it.each(['grant_id', 'run_id', 'device', 'session'] as const)('refuses lifecycle %s mismatch on every state', field => {
    const gate = makeGate()
    expect(gate.acceptResource(started)).toBe('activate')
    for (const row of vectors.resources) {
      expect(gate.acceptResource({ ...row, [field]: 'other/binding' })).toBe('refused-binding')
      expect(gate.activeRunId(grant.grant_id)).toBe(grant.run_id)
      expect(gate.enrollmentRevoked()).toBe(false)
    }
  })

  it('requires exact optional item binding for every state, including unbound grants', () => {
    const gate = makeGate()
    gate.acceptResource(started)
    for (const row of vectors.resources) {
      const { item_id: _item, ...missing } = row
      expect(gate.acceptResource(missing)).toBe('ignore-item-mismatch')
      expect(gate.acceptResource({ ...row, item_id: 'other/item' })).toBe('ignore-item-mismatch')
    }
    expect(gate.allows(command())).toBe(true)
    expect(gate.enrollmentRevoked()).toBe(false)
    const { item_id: _item, ...unbound } = grant
    const unboundGate = makeGate(() => unbound)
    expect(unboundGate.acceptResource(started)).toBe('ignore-item-mismatch')
    const { item_id: _rowItem, ...unboundRow } = started
    expect(unboundGate.acceptResource(unboundRow)).toBe('activate')
    expect(unboundGate.allows(command(unbound))).toBe(true)
    expect(unboundGate.allows(command())).toBe(false)
    expect(unboundGate.acceptResource(revoked)).toBe('ignore-item-mismatch')
    expect(unboundGate.enrollmentRevoked()).toBe(false)
  })

  it.each([
    null, undefined, {}, [], 'ended', { kind: 'unknown' },
    { ...started, extra: true }, { ...ended, state: 'unknown' },
    { ...revoked, sequence: 0 }, { ...revoked, sequence: -1 },
    { ...revoked, sequence: 1.5 }, { ...revoked, sequence: Number.MAX_SAFE_INTEGER + 1 },
    { ...revoked, sequence: null }, { ...revoked, item_id: null },
    { ...revoked, device: null }, { ...ended, updated_at: 'not-a-timestamp' },
  ])('refuses malformed or unknown resources without authority calls or termination: %j', raw => {
    let calls = 0
    const gate = makeGate(() => { calls += 1; return grant })
    gate.acceptResource(started)
    calls = 0
    expect(gate.acceptResource(raw)).toBe('refused-invalid')
    expect(calls).toBe(0)
    expect(gate.activeRunId(grant.grant_id)).toBe(grant.run_id)
    expect(gate.enrollmentRevoked()).toBe(false)
  })

  it('accepts strictly increasing starts, ignores positive reordered starts, and never resurrects finals', () => {
    const gate = makeGate()
    expect(gate.acceptResource({ ...started, sequence: 5 })).toBe('activate')
    expect(gate.acceptResource({ ...started, sequence: 4 })).toBe('ignore-stale-started')
    expect(gate.acceptResource({ ...started, sequence: 5 })).toBe('ignore-stale-started')
    expect(gate.acceptResource({ ...started, sequence: 6 })).toBe('activate')
    // Termination is final even if its sequence precedes the latest start.
    expect(gate.acceptResource(ended)).toBe('terminate')
    expect(gate.activeRunId(grant.grant_id)).toBeUndefined()
    expect(gate.acceptResource({ ...started, sequence: 100 })).toBe('ignore-final-grant')
    expect(gate.allows(command())).toBe(false)
    expect(gate.acceptResource(revoked)).toBe('remain-terminal')
    expect(gate.enrollmentRevoked()).toBe(true)
    expect(gate.activeRunId(grant.grant_id)).toBeUndefined()
  })

  it.each([ended, revoked])('makes a terminal row before any start permanently final: $state', terminal => {
    const gate = makeGate()
    expect(gate.acceptResource(terminal)).toBe('terminate')
    expect(gate.acceptResource({ ...started, sequence: 100 })).toBe('ignore-final-grant')
    expect(gate.activeRunId(grant.grant_id)).toBeUndefined()
    expect(gate.allows(command())).toBe(false)
  })

  it('ends only one grant, but verified revocation disables every active grant', () => {
    const other = { ...grant, grant_id: 'client-control-grant/other' }
    const gate = makeGate(id => id === grant.grant_id ? grant : id === other.grant_id ? other : undefined)
    gate.acceptResource(started)
    gate.acceptResource({ ...started, grant_id: other.grant_id })
    gate.acceptResource(ended)
    expect(gate.enrollmentRevoked()).toBe(false)
    expect(gate.activeRunId(other.grant_id)).toBe(other.run_id)
    expect(gate.allows(command(other))).toBe(true)
    expect(gate.acceptResource(revoked)).toBe('remain-terminal')
    expect(gate.enrollmentRevoked()).toBe(true)
    expect(gate.activeRunId(other.grant_id)).toBeUndefined()
    expect(gate.allows(command(other))).toBe(false)
    gate.acceptResource({ ...started, grant_id: 'client-control-grant/unknown' })
    expect(gate.enrollmentRevoked()).toBe(true)
  })

  it.each(['grant_id', 'run_id', 'device', 'session', 'item_id'] as const)('rechecks current authority and embedded grant %s', field => {
    let current: Wire.ClientControlGrant | undefined = grant
    const gate = makeGate(() => current)
    gate.acceptResource(started)
    expect(gate.allows(command())).toBe(true)
    const mismatched = { ...grant, [field]: 'other/binding' }
    expect(gate.allows({ ...command(), control_grant: mismatched })).toBe(false)
    current = mismatched
    expect(gate.allows(command())).toBe(false)
    expect(gate.acceptResource({ ...started, [field]: 'other/binding', sequence: 2 })).not.toBe('activate')
    current = undefined
    expect(gate.allows(command())).toBe(false)
  })

  it.each(['grant_id', 'run_id', 'device_id'] as const)('checks command %s against the active binding', field => {
    const gate = makeGate()
    gate.acceptResource(started)
    expect(gate.allows({ ...command(), [field]: 'other/binding' })).toBe(false)
  })

  it('requires command grant/run IDs, leaving session_actor projection checks to verified authority', () => {
    const gate = makeGate()
    gate.acceptResource(started)
    const { grant_id: _grantId, ...withoutGrantId } = command()
    const { run_id: _runId, ...withoutRunId } = command()
    expect(gate.allows(withoutGrantId)).toBe(false)
    expect(gate.allows(withoutRunId)).toBe(false)
    expect(gate.allows({ ...command(), session_actor: 'person/test/session/projected' })).toBe(true)
  })

  it.each(['grant_id', 'item_id', 'run_id', 'device', 'session', 'state', 'reason', 'sequence', 'id', 'kind', 'revision', 'updated_at'] as const)(
    'refuses null lifecycle field %s without state changes', field => {
      const gate = makeGate()
      gate.acceptResource(started)
      expect(gate.acceptResource({ ...revoked, [field]: null })).toBe('refused-invalid')
      expect(gate.activeRunId(grant.grant_id)).toBe(grant.run_id)
      expect(gate.enrollmentRevoked()).toBe(false)
    },
  )

  it('refuses otherwise-valid resource kinds outside the commands collection contract', () => {
    const operation = {
      kind: 'operation', id: 'operation/unit', revision: started.revision, updated_at: started.updated_at,
      component: 'daemon', severity: 'info', state: 'healthy', summary: 'Synthetic protocol example',
    }
    expect(() => Wire.decodeUnknownSync(Wire.Resource, 'strict')(operation)).not.toThrow()
    let calls = 0
    const gate = makeGate(() => { calls += 1; return grant })
    gate.acceptResource(started)
    calls = 0
    expect(gate.acceptFrame({ ...snapshot, items: [revoked, operation] })).toEqual(refusedFrame)
    expect(calls).toBe(0)
    expect(gate.activeRunId(grant.grant_id)).toBe(grant.run_id)
    expect(gate.enrollmentRevoked()).toBe(false)
  })

  it('does not interpret verb, screen, or budget policy as lifecycle authority', () => {
    const gate = makeGate()
    gate.acceptResource(started)
    expect(gate.allows(command({ ...grant, verbs: [], screens: [], max_commands: 1 }))).toBe(true)
  })

  it.each(['ended', 'revoked', 'expired', 'unknown', ''])(
    'refuses a command embedding nonactive grant state %j despite active lifecycle and authority', state => {
      const gate = makeGate()
      expect(gate.acceptResource(started)).toBe('activate')
      expect(gate.allows(command())).toBe(true)
      expect(gate.allows(command({ ...grant, state }))).toBe(false)
      expect(gate.activeRunId(grant.grant_id)).toBe(grant.run_id)
    },
  )

  it.each([
    null, undefined, {}, { ...snapshot, unexpected: true }, { ...snapshot, items: null },
    { ...snapshot, snapshot: { ...snapshot.snapshot, unexpected: true } },
    { ...snapshot, items: [ended, { ...revoked, sequence: 0 }] },
    { ...snapshot, items: [ended, { ...revoked, unexpected: true }] },
    { ...snapshot, items: [ended, { kind: 'unknown' }] },
    { ...changes, upserts: [ended, { ...revoked, item_id: null }] },
  ])('atomically refuses malformed frames before any authority callback: %j', raw => {
    let calls = 0
    const gate = makeGate(() => { calls += 1; return grant })
    gate.acceptResource(started)
    calls = 0
    expect(gate.acceptFrame(raw)).toEqual(refusedFrame)
    expect(calls).toBe(0)
    expect(gate.activeRunId(grant.grant_id)).toBe(grant.run_id)
    expect(gate.enrollmentRevoked()).toBe(false)
  })

  it.each([
    { ...snapshot, id: 'wrong-subscription' }, { ...snapshot, collection: 'attention' },
    { kind: 'resync', id: snapshot.id, collection: 'commands' },
    { kind: 'error', id: snapshot.id, collection: 'commands', message: 'failed' },
  ])('refuses non-command or nonmatching-subscription frames without authority calls: %j', raw => {
    let calls = 0
    const gate = makeGate(() => { calls += 1; return grant })
    expect(gate.acceptFrame(raw)).toEqual(refusedFrame)
    expect(calls).toBe(0)
    expect(gate.enrollmentRevoked()).toBe(false)
  })

  it.each(['grant_id', 'run_id', 'device', 'session', 'item_id'] as const)('prevalidates every frame binding before applying termination: %s', field => {
    const gate = makeGate()
    gate.acceptResource(started)
    for (const terminal of [ended, revoked]) {
      expect(gate.acceptFrame({ ...changes, upserts: [terminal, { ...revoked, [field]: 'other/binding' }] })).toEqual(refusedFrame)
      expect(gate.activeRunId(grant.grant_id)).toBe(grant.run_id)
      expect(gate.enrollmentRevoked()).toBe(false)
      expect(gate.allows(command())).toBe(true)
    }
  })

  it('prevalidates bindings introduced in the same frame even if authority changes between callbacks', () => {
    let calls = 0
    const gate = makeGate(() => {
      calls += 1
      return calls === 1 ? grant : { ...grant, run_id: 'mission-run/other' }
    })
    expect(gate.acceptFrame({ ...snapshot, items: [started, { ...revoked, run_id: 'mission-run/other' }] })).toEqual(refusedFrame)
    expect(gate.activeRunId(grant.grant_id)).toBeUndefined()
    expect(gate.enrollmentRevoked()).toBe(false)
  })

  it('ignores command rows and collection removals rather than treating absence as termination', () => {
    const gate = makeGate()
    gate.acceptResource(started)
    expect(gate.acceptFrame({ ...snapshot, items: [Schema.encodeSync(Wire.DeviceCommand)(command())] })).toEqual({ accepted: true, decisions: [] })
    expect(gate.acceptFrame({ ...changes, upserts: [], removes: [started.id] })).toEqual({ accepted: true, decisions: [] })
    expect(gate.allows(command())).toBe(true)
    expect(gate.enrollmentRevoked()).toBe(false)
  })
})
