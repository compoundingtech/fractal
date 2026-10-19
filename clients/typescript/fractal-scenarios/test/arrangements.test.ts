import { afterEach, describe, expect, it } from 'vitest'
import { FractalClient, type CollectionFrame } from '@fractal/client'
import { Arrangement, ArrangementPage, CollectionFrame as Frame, decodeUnknownSync } from '@fractal/client/schema'

import { ANCHOR_MS, loadWorld, type World } from '../src/index.ts'
import { createReplay, manualClock, type Replay } from '../src/replay/index.ts'

const replays: Replay[] = []
afterEach(() => { for (const replay of replays.splice(0)) replay.close() })
const setup = (world: World) => {
  const clock = manualClock(world.now)
  const replay = createReplay(world, { clock })
  replays.push(replay)
  const client = new FractalClient({ baseUrl: 'http://scenario.invalid', fetchImpl: replay.fetch })
  return { world, clock, replay, client }
}
const fleet = () => loadWorld('fleet-mid-refactor', { now: ANCHOR_MS })

describe('read-only sidebar arrangements', () => {
  it('grants onboarding folder discovery with an empty inventory', async () => {
    const h = setup(loadWorld('first-run-onboarding', { now: ANCHOR_MS }))
    const discovery = (await h.client.capabilities()).value
    expect(discovery.capabilities).toContainEqual({ id: 'arrangements', version: 1, state: 'granted' })
    expect(discovery.capabilities).toContainEqual({ id: 'read.arrangements', version: 0, state: 'granted' })
    expect(discovery.capabilities.some((row) => row.id === 'control.arrangements')).toBe(false)
    const person = discovery.session_actor.split('/session/')[0]!
    const page = (await h.client.arrangementsList(person)).value
    decodeUnknownSync(ArrangementPage, 'strict')(page)
    expect(page.items).toEqual([])
    expect(page.page.has_more).toBe(false)
    expect(h.replay.served()).toContain('roster')
  })

  it('serves a decodable Sidebar whose placements refer to roster agents', async () => {
    const h = setup(fleet())
    const owner = h.world.cast.people[0]!.id
    const page = (await h.client.arrangementsList(owner)).value
    decodeUnknownSync(ArrangementPage, 'strict')(page)
    expect(page.items).toHaveLength(1)
    const sidebar = page.items[0]!
    expect(sidebar.owner).toBe(owner)
    expect(sidebar.body.name.value).toBe('Sidebar')
    expect(Object.keys(sidebar.body.folders)).toHaveLength(3)
    expect(Object.keys(sidebar.body.placements)).toHaveLength(3)
    for (const [subject, placement] of Object.entries(sidebar.body.placements)) {
      expect(h.world.slices.roster.state.agents.some((agent) => agent.id === subject)).toBe(true)
      expect(placement.value.folder).not.toBeNull()
      expect(Object.hasOwn(sidebar.body.folders, placement.value.folder!)).toBe(true)
    }
    const detail = (await h.client.arrangementsGet(owner.slice('person/'.length), sidebar.id.split('/').at(-1)!)).value
    decodeUnknownSync(Arrangement, 'strict')(detail)
    expect(detail).toEqual(sidebar)
    await expect(h.client.arrangementsGet(owner.slice('person/'.length), '00000000-0000-7000-8000-000000000000')).rejects.toMatchObject({ status: 404, response: { code: 'not-found' } })
    const other = h.world.cast.people[1]!.id
    const forbidden = { status: 403, response: { code: 'forbidden', message: 'cross-person arrangements require delegated authority', retryable: false } }
    await expect(h.client.arrangementsList(other)).rejects.toMatchObject(forbidden)
    await expect(h.client.arrangementsGet(other.slice('person/'.length), sidebar.id.split('/').at(-1)!)).rejects.toMatchObject(forbidden)
  })

  it('returns HTTP 422 for arrangement validation without changing generic limit errors', async () => {
    const h = setup(fleet())
    const owner = h.world.cast.people[0]!.id
    const query = new URLSearchParams({ person: owner }).toString()
    const paths = [
      '/arrangements',
      '/arrangements?person=invalid-owner',
      `/arrangements/${owner.slice('person/'.length)}/not-a-uuid`,
      `/arrangements?${query}&actor=invalid-actor`,
      `/arrangements?${query}&history=true`,
    ]
    for (const path of paths) {
      const response = await h.replay.fetch(`http://scenario.invalid/v1/client${path}`)
      expect(response.status, path).toBe(422)
      expect(await response.json()).toMatchObject({ code: 'validation-failed', retryable: false })
    }
    const invalidLimit = await h.replay.fetch(`http://scenario.invalid/v1/client/arrangements?${query}&limit=0`)
    expect(invalidLimit.status).toBe(400)
    const frames: CollectionFrame[] = []
    const stream = await h.client.collectionStream({ socket: h.replay.socket, onFrame: (frame) => {
      decodeUnknownSync(Frame, 'strict')(frame)
      frames.push(frame)
    } })
    const other = h.world.cast.people[1]!.id
    stream.subscribeArrangements('wrong-owner', owner, 1, `arrangement/${other}/00000000-0000-7000-8000-000000000000`)
    h.clock.advance(0)
    expect(frames.find((frame) => frame.id === 'wrong-owner')).toMatchObject({
      kind: 'error', code: 'validation-failed', message: 'selected arrangement owner must match person', retryable: false,
    })
    stream.close()
  })

  it('filters owner-wide and exact subscriptions and emits no folder changes', async () => {
    const h = setup(fleet())
    const owner = h.world.cast.people[0]!.id
    const sidebar = h.world.slices.roster.state.arrangements![0]!
    const frames: CollectionFrame[] = []
    const stream = await h.client.collectionStream({ socket: h.replay.socket, onFrame: (frame) => {
      decodeUnknownSync(Frame, 'strict')(frame)
      frames.push(frame)
    } })
    stream.subscribeArrangements('owner', owner, 1)
    stream.subscribeArrangements('exact', owner, 1, sidebar.id)
    stream.subscribeArrangements('other', h.world.cast.people[1]!.id, 1)
    stream.subscribeArrangements('missing', owner, 1, `arrangement/${owner}/00000000-0000-7000-8000-000000000000`)
    h.clock.advance(0)
    for (const id of ['owner', 'exact']) expect(frames.find((frame) => frame.kind === 'snapshot' && frame.id === id)).toMatchObject({ items: [sidebar] })
    expect(frames.find((frame) => frame.kind === 'snapshot' && frame.id === 'missing')).toMatchObject({ items: [] })
    expect(frames.find((frame) => frame.id === 'other')).toMatchObject({
      kind: 'error', collection: 'arrangements', code: 'forbidden',
      message: 'cross-person arrangements require delegated authority', retryable: false,
    })
    h.clock.advance(20_000)
    expect(frames.some((frame) => frame.kind === 'changes' && ['owner', 'exact', 'other', 'missing'].includes(frame.id ?? ''))).toBe(false)
    expect(h.replay.served()).toContain('roster')
    stream.close()
  })

  it('advertises the server ungranted feature shape for the refusal variant', async () => {
    const h = setup(loadWorld('failed-sync-arrangements-absent', { now: ANCHOR_MS }))
    const discovery = (await h.client.capabilities()).value
    expect(discovery.capabilities).toContainEqual({ id: 'arrangements', version: 1, state: 'ungranted' })
    expect(discovery.capabilities).toContainEqual({ id: 'read.arrangements', version: 0, state: 'ungranted' })
    expect(discovery.capabilities.some((row) => row.id === 'arrangements' && row.version === 1 && row.state === 'granted')).toBe(false)
    // Same guard as fractal-web requireArrangements: feature id, exact version and granted state.
    const requireArrangements = () => {
      if (!discovery.capabilities.some((row) => row.id === 'arrangements' && row.version === 1 && row.state === 'granted')) {
        throw new TypeError('Granted arrangements capability version 1 is required; older pairings may need renewal')
      }
    }
    expect(requireArrangements).toThrow(TypeError)
    const owner = h.world.cast.people[0]!.id
    const forbidden = { status: 403, response: { code: 'forbidden',
      message: 'the authenticated client session is not granted `read.arrangements`', retryable: false } }
    await expect(h.client.arrangementsList(owner)).rejects.toMatchObject(forbidden)
    await expect(h.client.arrangementsGet(owner.slice('person/'.length), '00000000-0000-7000-8000-000000000000')).rejects.toMatchObject(forbidden)
    const frames: CollectionFrame[] = []
    const stream = await h.client.collectionStream({ socket: h.replay.socket, onFrame: (frame) => {
      decodeUnknownSync(Frame, 'strict')(frame)
      frames.push(frame)
    } })
    stream.subscribeArrangements('denied', owner, 1)
    h.clock.advance(0)
    expect(frames.find((frame) => frame.id === 'denied')).toMatchObject({
      kind: 'error', collection: 'arrangements', code: 'forbidden',
      message: forbidden.response.message, retryable: false,
    })
    expect(frames.some((frame) => frame.kind === 'snapshot')).toBe(false)
    stream.close()
  })

  it('rejects arrangement edits instead of accepting replay writes', async () => {
    const h = setup(fleet())
    const sidebar = h.world.slices.roster.state.arrangements![0]!
    const response = await h.replay.fetch('http://scenario.invalid/v1/client/actions', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ api_version: 'st3.client.v0', type: 'arrangement.edit', id: 'action/scenario-sidebar-edit',
        parameters: { owner: sidebar.owner, subject: sidebar.id, operations: [{ op: 'rename', name: 'Changed' }] } }),
    })
    expect(response.status).toBe(405)
    expect(await response.json()).toMatchObject({ code: 'unsupported-capability', retryable: false })
    expect(h.replay.actions).toEqual([])
    expect((await h.client.arrangementsList(sidebar.owner)).value.items).toEqual([sidebar])
  })

  it('preserves explicit roster inventory overrides', async () => {
    const world = fleet()
    const roster = { ...world.slices.roster, state: { ...world.slices.roster.state, arrangements: [] } }
    const h = setup(world.with({ roster }))
    expect(h.world.slices.roster.state.arrangements).toEqual([])
    expect((await h.client.arrangementsList(world.cast.people[0]!.id)).value.items).toEqual([])
  })

  it('keeps UUIDs stable while shifting timestamps and bounds nested huge inventory', () => {
    const original = fleet().slices.roster.state.arrangements![0]!
    const shifted = loadWorld('fleet-mid-refactor', { now: ANCHOR_MS + 60_000 }).slices.roster.state.arrangements![0]!
    expect(shifted.id).toBe(original.id)
    expect(shifted.body).toEqual(original.body)
    expect(Date.parse(shifted.updated_at) - Date.parse(original.updated_at)).toBe(60_000)
    const huge = loadWorld('huge', { now: ANCHOR_MS })
    const sidebar = huge.slices.roster.state.arrangements![0]!
    decodeUnknownSync(Arrangement, 'strict')(sidebar)
    expect(Object.keys(sidebar.body.folders)).toHaveLength(57)
    expect(Object.keys(sidebar.body.placements)).toHaveLength(1_000)
    expect(Object.values(sidebar.body.folders).some((folder) => folder.position.value.parent !== null && sidebar.body.folders[folder.position.value.parent]!.position.value.parent !== null)).toBe(true)
    const counts = new Map<string, number>()
    const count = (revision: string): void => { counts.set(revision, (counts.get(revision) ?? 0) + 1) }
    count(sidebar.body.name.revision) // arrangement creation
    for (const folder of Object.values(sidebar.body.folders)) count(folder.name.revision) // folder.create sets both registers
    for (const placement of Object.values(sidebar.body.placements)) count(placement.revision)
    expect(counts.size).toBe(5)
    expect(Math.max(...counts.values())).toBeLessThanOrEqual(256)
    expect(sidebar.revision).toBe('arrangement-5')
  })
})
