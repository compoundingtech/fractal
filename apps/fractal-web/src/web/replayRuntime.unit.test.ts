import { catalog } from '@smalltalk/st3-scenarios'
import { describe, expect, it } from 'vitest'
import { createDevReplay, selectReplayWorld } from './replayRuntime.ts'

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
