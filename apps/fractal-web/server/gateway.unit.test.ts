import { describe, expect, it } from 'vitest'
import { participantsFeature, withClientFeatures } from './gateway.mts'

describe('client features the gateway asks for', () => {
  it('adds message participants to the WebSocket defaults the browser cannot send', () => {
    expect(withClientFeatures(undefined)).toBe(`custom-subjects.v1, conversation-blocks.v1, model-drift-attention.v1, ${participantsFeature}`)
  })
  it('keeps every feature the SDK asked for and adds participants once', () => {
    expect(withClientFeatures('custom-subjects.v1, conversation-blocks.v1, messages-paging.v2')).toBe(`custom-subjects.v1, conversation-blocks.v1, messages-paging.v2, ${participantsFeature}`)
    expect(withClientFeatures(`conversation-blocks.v1, ${participantsFeature}`)).toBe(`conversation-blocks.v1, ${participantsFeature}`)
    expect(withClientFeatures(['a.v1', 'b.v1'])).toBe(`a.v1, b.v1, ${participantsFeature}`)
  })
})
