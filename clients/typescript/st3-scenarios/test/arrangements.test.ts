import { afterEach, describe, expect, it } from 'vitest'
import { St3Client, type CollectionFrame } from '@smalltalk/st3-client'
import { Arrangement, ArrangementPage, CollectionFrame as Frame, decodeUnknownSync } from '@smalltalk/st3-client/schema'

import { ANCHOR_MS, loadWorld, type World } from '../src/index.ts'
import { createReplay, manualClock, type Replay } from '../src/replay/index.ts'

const replays: Replay[] = []
afterEach(() => { for (const replay of replays.splice(0)) replay.close() })
const setup = (world: World) => {
  const clock = manualClock(world.now)
  const replay = createReplay(world, { clock })
  replays.push(replay)
  const client = new St3Client({ baseUrl: 'http://scenario.invalid', fetchImpl: replay.fetch })
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
    expect((await h.client.arrangementsList(h.world.cast.people[1]!.id)).value.items).toEqual([])
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
    for (const id of ['other', 'missing']) expect(frames.find((frame) => frame.kind === 'snapshot' && frame.id === id)).toMatchObject({ items: [] })
    h.clock.advance(20_000)
    expect(frames.some((frame) => frame.kind === 'changes' && ['owner', 'exact', 'other', 'missing'].includes(frame.id ?? ''))).toBe(false)
    expect(h.replay.served()).toContain('roster')
    stream.close()
  })

  it('advertises the server ungranted feature shape for the refusal variant', async () => {
    const h = setup(loadWorld('failed-sync-arrangements-absent', { now: ANCHOR_MS }))
    const discovery = (await h.client.capabilities()).value
    expect(discovery.capabilities).toContainEqual({ id: 'arrangements', version: 1, state: 'ungranted' })
    expect(discovery.capabilities.some((row) => row.id === 'arrangements' && row.version === 1 && row.state === 'granted')).toBe(false)
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
  })
})
