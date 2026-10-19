import { catalog } from '@smalltalk/st3-scenarios'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { folders, installPageFolders } from '../folders/client.ts'
import { createDevReplay, selectReplayWorld } from './replayRuntime.ts'

const originalFolders = folders
afterEach(() => {
  installPageFolders(originalFolders)
  vi.unstubAllGlobals()
})

describe('dev replay selection', () => {
  it('defaults to live, including unrelated query parameters', () => {
    expect(selectReplayWorld('')).toBeUndefined()
    expect(createDevReplay('?search=agents')).toBeUndefined()
  })
  it('selects an explicit catalog world', () => {
    expect(selectReplayWorld(`?scenario=${catalog[0]!.id}`)).toBe(catalog[0]!.id)
    const replay = createDevReplay(`?scenario=${catalog[0]!.id}`)!
    expect(replay.socket).toBeTypeOf('function')
    replay.close()
  })
  it.each(['missing-world', ''])('rejects unknown world %j with the available choices', (id) => {
    expect(() => selectReplayWorld(`?scenario=${id}`)).toThrow('Unknown Fractal replay scenario')
    expect(() => selectReplayWorld(`?scenario=${id}`)).toThrow(catalog[0]!.id)
  })
})

it('routes folder discovery through replay, surfacing its missing arrangement capability without network', async () => {
  vi.stubGlobal('location', { origin: 'http://localhost' })
  const network = vi.fn(() => { throw new Error('Unexpected real request') })
  vi.stubGlobal('fetch', network)
  const replay = createDevReplay('?scenario=first-run-onboarding')!
  const registry = AtomRegistry.make()
  const release = registry.mount(folders)
  try {
    await vi.waitFor(() => expect(registry.get(folders).phase).toBe('unavailable'))
    expect(registry.get(folders).detail).toContain('Granted arrangements capability version 1 is required')
    expect(network).not.toHaveBeenCalled()
  } finally {
    release()
    registry.dispose()
    replay.close()
  }
})
