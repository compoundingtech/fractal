import { describe, expect, it } from 'vitest'
import { isClientControlForeground } from './foreground.ts'

describe('client-control foreground', () => {
  it.each([
    ['visible', true, true],
    ['visible', false, false],
    ['hidden', true, false],
    ['hidden', false, false],
  ] as const)('visibility=%s focus=%s permits=%s', (visibilityState, focus, expected) => {
    expect(isClientControlForeground({ visibilityState, hasFocus: () => focus })).toBe(expected)
  })

  it('does not retain a previous focus verdict', () => {
    let focused = true
    const document = { visibilityState: 'visible' as const, hasFocus: () => focused }
    expect(isClientControlForeground(document)).toBe(true)
    focused = false
    expect(isClientControlForeground(document)).toBe(false)
  })
})
