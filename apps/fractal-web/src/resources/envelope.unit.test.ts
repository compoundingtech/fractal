import { describe, expect, it } from 'vitest'

import { decodeEnvelope, parseSchemaId } from './envelope.ts'
import { missionEnvelope } from './fixtures.ts'

describe('subject envelope schema spellings', () => {
  it('accepts both namespaces and preserves routing parts and the wire spelling', () => {
    for (const prefix of ['st3', 'fractal']) {
      for (const suffix of ['mission@1', 'resource:vcs.pull-request@12']) {
        const schema = `${prefix}.${suffix}`
        expect(decodeEnvelope({ ...missionEnvelope, schema })).toMatchObject({
          _tag: 'Envelope', envelope: { schema },
        })
        expect(parseSchemaId(schema)).toEqual({
          name: schema.slice(0, schema.lastIndexOf('@')),
          family: suffix.startsWith('resource:') ? 'resource' : 'mission',
          kind: suffix.startsWith('resource:') ? 'vcs.pull-request' : undefined,
          major: suffix.startsWith('resource:') ? 12 : 1,
        })
      }
    }
  })

  it('refuses unknown namespaces and still refuses malformed schema ids', () => {
    for (const schema of ['smalltalk.mission@1', 'fractalx.mission@1', 'fractal.mission@0', 'fractal.mission@01', 'fractal.Mission@1', 'fractal.resource:@1', 'fractal.mission@1.extra']) {
      expect(parseSchemaId(schema)).toBeUndefined()
      expect(decodeEnvelope({ ...missionEnvelope, schema })._tag).toBe('Malformed')
    }
  })
})
