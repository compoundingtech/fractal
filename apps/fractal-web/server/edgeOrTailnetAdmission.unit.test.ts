import type { IncomingMessage } from 'node:http'
import { describe, expect, it, vi } from 'vitest'
import { edgeOrTailnetAdmission } from './host.mts'
import { devAdmission } from './devAdmission.mts'

const origin = 'https://atlas.example.ts.net'
const host = 'atlas.example.ts.net'
const edgeHost = 'preview.example.net'
const login = 'robin@example.com'
const listener = '127.0.0.1:7411'
const request = (headers: IncomingMessage['headers'], method = 'GET') => ({ headers, method }) as IncomingMessage
const edgeHeaders = { host: listener, 'x-forwarded-host': edgeHost, 'x-forwarded-proto': 'https' }
// The spy verifies dispatch independently of the concrete edge policy tested below.
const setup = () => {
  const edge = vi.fn((req: IncomingMessage) => req.headers.host === listener &&
    req.headers['x-forwarded-host'] === edgeHost && req.headers['x-forwarded-proto'] === 'https')
  return { edge, admit: edgeOrTailnetAdmission({ origin, logins: [login], edge }) }
}

describe('edgeOrTailnetAdmission', () => {
  it('delegates existing edge HTTP and WebSocket requests unchanged', () => {
    const { edge, admit } = setup()
    for (const headers of [edgeHeaders, { ...edgeHeaders, origin: `http://${listener}`, upgrade: 'websocket' }]) {
      const req = request(headers)
      expect(admit(req)).toBe(true)
      expect(edge).toHaveBeenLastCalledWith(req)
    }
  })

  it('admits the canonical host only with an allowed login and exact Origin', () => {
    const { edge, admit } = setup()
    for (const upgrade of [undefined, 'websocket']) {
      expect(admit(request({ host, origin, 'tailscale-user-login': login, upgrade }))).toBe(true)
      for (const missingOrOther of [undefined, 'blair@example.com'])
        expect(admit(request({ host, origin, 'tailscale-user-login': missingOrOther, upgrade }))).toBe(false)
      for (const wrongOrigin of ['https://elsewhere.example', `${origin}/`, 'null'])
        expect(admit(request({ host, origin: wrongOrigin, 'tailscale-user-login': login, upgrade }))).toBe(false)
    }
    expect(edge).not.toHaveBeenCalled()
  })

  it('retains tailnet semantics for Origin-less HTTP reads and upgrades', () => {
    const { admit } = setup()
    const headers = { host, 'tailscale-user-login': login }
    expect(admit(request(headers))).toBe(true)
    expect(admit(request(headers, 'POST'))).toBe(false)
    expect(admit(request({ ...headers, upgrade: 'websocket' }))).toBe(false)
  })

  it('does not fall back to the edge policy for the canonical or an unknown host', () => {
    const { admit } = setup()
    expect(admit(request({ ...edgeHeaders, host, origin }))).toBe(false)
    expect(admit(request({ ...edgeHeaders, host: 'unknown.example', origin }))).toBe(false)
    expect(admit(request({ host: edgeHost, origin: `https://${edgeHost}` }))).toBe(false)
  })

  it('rejects Serve-rewritten edge forgery even for tagged nodes with no identity', () => {
    const { edge, admit } = setup()
    // Serve preserves the supplied Host but overwrites X-Forwarded-Host with it.
    for (const forgedHost of [listener, edgeHost, 'unknown.example'])
      expect(admit(request({ ...edgeHeaders, host: forgedHost, 'x-forwarded-host': forgedHost }))).toBe(false)
    expect(edge).not.toHaveBeenCalled()
  })

  it('rejects identity-bearing requests before the login-free edge callback', () => {
    const { edge, admit } = setup()
    for (const name of ['tailscale-user-login', 'tailscale-user-name', 'tailscale-headers-info'])
      expect(admit(request({ ...edgeHeaders, [name]: login }))).toBe(false)
    expect(edge).not.toHaveBeenCalled()
  })

  it('fails startup for an empty allowlist or invalid published origin', () => {
    const edge = () => true
    expect(() => edgeOrTailnetAdmission({ origin, logins: [], edge })).toThrow(/login/)
    expect(() => edgeOrTailnetAdmission({ origin: `${origin}/`, logins: [login], edge })).toThrow(/origin/)
  })

  it('returns the exact previous policy when WF_PUBLIC_ORIGIN is unset', () => {
    const edge = vi.fn(() => true)
    const admit = edgeOrTailnetAdmission({ origin: undefined, logins: [], edge })
    expect(admit).toBe(edge)
    expect(admit(request({ host: 'anything.example' }))).toBe(true)
  })

  it('keeps the public edge admission and tailnet admission disjoint', () => {
    const edge = devAdmission({ edgeHost, edgeAliases: ['alias.example.net'], port: () => 7411 })
    const admit = edgeOrTailnetAdmission({ origin, logins: ['reader'], edge })
    const incoming = (headers: IncomingMessage['headers']) =>
      ({ headers, rawHeaders: [], method: 'GET', url: '/', socket: { remoteAddress: '127.0.0.1' } }) as IncomingMessage
    expect(admit(incoming(edgeHeaders))).toBe(true)
    expect(admit(incoming({ ...edgeHeaders, origin: `http://${listener}`, upgrade: 'websocket' }))).toBe(true)
    expect(admit(incoming({ ...edgeHeaders, 'x-forwarded-host': 'alias.example.net' }))).toBe(true)
    expect(admit(incoming({ host, origin, 'tailscale-user-login': 'reader' }))).toBe(true)
    for (const headers of [
      { ...edgeHeaders, host },
      { ...edgeHeaders, host: 'unknown.example' },
      { ...edgeHeaders, 'x-forwarded-host': host },
      { ...edgeHeaders, 'tailscale-user-login': 'reader' },
      { ...edgeHeaders, 'tailscale-node-tags': 'tag:worker' },
      { ...edgeHeaders, 'x-forwarded-host': listener },
      { host, origin, 'tailscale-user-login': 'unknown' },
    ]) expect(admit(incoming(headers))).toBe(false)
  })
})
