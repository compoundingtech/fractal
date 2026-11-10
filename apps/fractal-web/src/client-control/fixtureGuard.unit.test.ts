import { describe, it } from '@effect/vitest'
import { expect } from 'vitest'
import { guardFixtureBoundary, type FixtureGuardInput } from './fixtureGuard.ts'

const approved: FixtureGuardInput = {
  origin: 'http://127.0.0.1:43127', approvedOrigin: 'http://127.0.0.1:43127',
  normalOrigins: ['http://127.0.0.1:5173'],
  gateway: 'unix:/isolated/client-control-fixture/run/fractal-client.sock',
  resolvedGatewayPath: '/isolated/client-control-fixture/run/fractal-client.sock',
  approvedClientSocketPath: '/isolated/client-control-fixture/run/fractal-client.sock',
  approvedInstanceRoot: '/isolated/client-control-fixture',
  normalGatewayPaths: ['/run/fractal-client.sock'],
}

describe('fixture boundary (configuration only)', () => {
  it('accepts only the reserved exact origin and resolved approved client socket', () => {
    expect(guardFixtureBoundary(approved)).toEqual({
      origin: approved.origin, socketPath: approved.resolvedGatewayPath, instance: 'client-control-fixture',
    })
    expect(guardFixtureBoundary({ ...approved, gateway: approved.resolvedGatewayPath }).socketPath).toBe(approved.resolvedGatewayPath)
  })

  it.each([
    'http://localhost:43127', 'http://[::1]:43127', 'http://0.0.0.0:43127',
    'https://127.0.0.1:43127', 'http://127.0.0.1', 'http://127.0.0.1:43127/',
    'http://127.0.0.1:43127?fixture=1', 'http://127.0.0.1:43127#fixture',
    'http://user@127.0.0.1:43127', 'http://127.0.0.2:43127',
  ])('refuses nonexact or nonapproved origin %s', origin => {
    expect(() => guardFixtureBoundary({ ...approved, origin })).toThrow()
  })

  it('refuses a reserved origin also used by a normal session', () => {
    expect(() => guardFixtureBoundary({ ...approved, normalOrigins: [approved.origin] })).toThrow()
  })

  it.each([
    undefined, '', '/run/fractal-client.sock', 'unix:/isolated/client-control-fixture/run/api.sock',
    'unix:/isolated/client-control-fixture/run/../fractal-client.sock',
    'unix:/isolated/client-control-fixture//run/fractal-client.sock',
    'unix:/isolated/client-control-fixture/run/fractal-client.sock?fixture=1',
    'unix:/isolated/client-control-fixture-other/run/fractal-client.sock',
    'https://127.0.0.1:43127',
  ])('refuses absent, privileged, ambiguous or non-fixture gateway %s', gateway => {
    expect(() => guardFixtureBoundary({ ...approved, gateway })).toThrow()
  })

  it('rejects symlink resolution disagreement and normal gateway aliases', () => {
    expect(() => guardFixtureBoundary({ ...approved, resolvedGatewayPath: '/run/fractal-client.sock' })).toThrow()
    expect(() => guardFixtureBoundary({ ...approved, normalGatewayPaths: [approved.resolvedGatewayPath] })).toThrow()
  })

  it('rejects an unrelated root even when all socket fields agree', () => {
    const socket = '/isolated/ordinary/run/fractal-client.sock'
    expect(() => guardFixtureBoundary({ ...approved, approvedInstanceRoot: '/isolated/ordinary',
      gateway: socket, resolvedGatewayPath: socket, approvedClientSocketPath: socket })).toThrow()
  })
})
