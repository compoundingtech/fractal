// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest'
import { paletteRecentsKey } from './LiveCommandPalette.tsx'

vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (variables: unknown) => variables, createTheme: () => ({}), keyframes: () => 'animation', props: () => ({}) }))

describe('palette recents persistence scope', () => {
  it('uses a deterministic opaque key instead of a connection URL', () => {
    const gateway = 'https://gateway.example:443/fleet?user=user&login=password&token=private-token'
    const key = paletteRecentsKey(gateway)
    expect(key).toBe(paletteRecentsKey(gateway))
    expect(key).toMatch(/^palette\.recents\.[0-9a-f]+$/)
    for (const identity of ['user', 'password', 'gateway.example', 'fleet', 'private-token']) {
      expect(key).not.toContain(identity)
    }
  })

  it('keeps different gateways and source modes in separate scopes', () => {
    const scopes = ['http://first.invalid', 'http://second.invalid', 'fixture', 'live']
    expect(new Set(scopes.map(paletteRecentsKey)).size).toBe(scopes.length)
  })
})
