import { describe, expect, it } from 'vitest'

import { ANCHOR_MS, catalog, loadWorld } from '../src/index.ts'
import { foldSlice } from '../src/kit/fold.ts'
import { decodeSlice } from '../scripts/decode.ts'
import { sliceFile } from '../scripts/emit.ts'
import { scanIdentities } from '../scripts/scan.ts'

describe('canonical inventories', () => {
  it('authors explicit inventories in every world', () => {
    for (const { id } of catalog) {
      const { roster, details } = loadWorld(id, { now: ANCHOR_MS }).slices
      expect(Array.isArray(roster.state.sessions)).toBe(true)
      expect(Array.isArray(roster.state.devices)).toBe(true)
      expect(Array.isArray(details.state.launches)).toBe(true)
      expect(Array.isArray(details.state.launch_variants)).toBe(true)
    }
  })

  it('rejects inventory identities outside the synthetic namespace', () => {
    for (const kind of ['device', 'launch', 'launch-variant']) {
      expect(scanIdentities('inventory.json', JSON.stringify({ id: `${kind}/outside` }))).toHaveLength(1)
      expect(scanIdentities('inventory.json', JSON.stringify({ id: `${kind}/scenario-fleet-inventory` }))).toEqual([])
    }
  })

  it('strict-decodes each inventory rather than treating it as opaque data', () => {
    const { id, slices: { roster, details } } = loadWorld('fleet-mid-refactor', { now: ANCHOR_MS })
    const badRoster = { ...roster, state: { ...roster.state,
      sessions: [{ ...roster.state.sessions[0]!, started_at: 'invalid' }],
      devices: [{ ...roster.state.devices[0]!, expires_at: 'invalid' }],
    } }
    const badDetails = { ...details, state: { ...details.state,
      launches: [{ ...details.state.launches[0]!, updated_at: 'invalid' }],
      launch_variants: [{ ...details.state.launch_variants[0]!, updated_at: 'invalid' }],
    } }
    expect(decodeSlice(id, badRoster).map(({ pointer }) => pointer)).toEqual(['/state/sessions/0', '/state/devices/0'])
    expect(decodeSlice(id, badDetails).map(({ pointer }) => pointer)).toEqual(['/state/launches/0', '/state/launch_variants/0'])
  })

  it('connects fleet sessions to conversation owners and variants to their launch', () => {
    const { roster, conversation, details } = loadWorld('fleet-mid-refactor', { now: ANCHOR_MS }).slices
    for (const thread of conversation.state.threads) {
      const session = roster.state.sessions.find(({ id }) => id === thread.session_id)
      expect(session?.owner_id).toBe(thread.agent)
      expect(roster.state.agents.some(({ id }) => id === session?.owner_id)).toBe(true)
    }
    expect(roster.state.devices).toHaveLength(2)
    expect(details.state.launches).toHaveLength(1)
    expect(details.state.launch_variants).toHaveLength(2)
    for (const variant of details.state.launch_variants) {
      const launch = details.state.launches.find(({ id }) => id === variant.launch_id)
      expect(launch?.variants).toContain(variant.id)
    }
  })

  it('includes inventory timestamps in portable fixture pointers and rebases them', () => {
    const anchor = loadWorld('fleet-mid-refactor', { now: ANCHOR_MS })
    const shifted = loadWorld('fleet-mid-refactor', { now: ANCHOR_MS + 86_400_000 })
    const roster = sliceFile(anchor, anchor.slices.roster)
    const details = sliceFile(anchor, anchor.slices.details)
    const pointers = roster.times.map(({ pointer }) => pointer)
    expect(pointers).toContain('/state/sessions/0/started_at')
    expect(pointers).toContain('/state/sessions/0/updated_at')
    expect(pointers).toContain('/state/devices/0/expires_at')
    expect(details.times.map(({ pointer }) => pointer)).toContain('/state/launches/0/updated_at')
    expect(details.times.map(({ pointer }) => pointer)).toContain('/state/launch_variants/0/updated_at')
    expect(Date.parse(shifted.slices.roster.state.devices[0]!.expires_at) - Date.parse(anchor.slices.roster.state.devices[0]!.expires_at)).toBe(86_400_000)
  })

  it('strict-decodes inventories and folds updates/removals into their owning slices', () => {
    const { id, slices: { roster, details } } = loadWorld('fleet-mid-refactor', { now: ANCHOR_MS })
    expect(decodeSlice(id, roster)).toEqual([])
    expect(decodeSlice(id, details)).toEqual([])
    const session = { ...roster.state.sessions[0]!, state: 'completed' as const, revision: 's-2' }
    const variant = { ...details.state.launch_variants[0]!, status: 'approved' as const, revision: 'lv-2' }
    const device = { ...roster.state.devices[1]!, state: 'revoked' as const, revision: 'd-2' }
    const launch = { ...details.state.launches[0]!, phase: 'approved' as const, revision: 'l-2' }
    const nextRoster = foldSlice({ ...roster, timeline: [{ _tag: 'changes', at_ms: 1, store: 1, upserts: [session, device], removes: [roster.state.devices[0]!.id] }] }, 1)
    const nextDetails = foldSlice({ ...details, timeline: [{ _tag: 'changes', at_ms: 1, store: 1, upserts: [variant, launch], removes: [] }] }, 1)
    expect(nextRoster.state.sessions.find(({ id }) => id === session.id)).toEqual(session)
    expect(nextRoster.state.devices).toHaveLength(1)
    expect(nextDetails.state.launch_variants.find(({ id }) => id === variant.id)).toEqual(variant)
    expect(nextRoster.state.devices[0]).toEqual(device)
    expect(nextDetails.state.launches[0]).toEqual(launch)
  })
})
