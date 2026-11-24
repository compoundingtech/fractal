import type { IncomingMessage } from 'node:http'

import { describe, expect, it } from 'vitest'

import { devAdmission } from './devAdmission.mts'

const request = (headers: IncomingMessage['headers'], rawHeaders: string[] = [], remoteAddress = '127.0.0.1', url = '/') =>
  ({ headers, rawHeaders, socket: { remoteAddress }, url }) as IncomingMessage
const admit = devAdmission({ port: () => 5173 })

describe('devAdmission', () => {
  it('admits the Vite origin for HTTP and WebSocket upgrades', () => {
    expect(admit(request({ host: '127.0.0.1:5173', origin: 'http://127.0.0.1:5173' }))).toBe(true)
    expect(admit(request({ host: 'localhost:5173', origin: 'http://localhost:5173', upgrade: 'websocket' }))).toBe(true)
  })

  it('admits Origin-less HTTP but not Origin-less upgrades', () => {
    expect(admit(request({ host: '127.0.0.1:5173' }))).toBe(true)
    expect(admit(request({ host: '127.0.0.1:5173', upgrade: 'websocket' }))).toBe(false)
  })

  it('rejects foreign, null and other-port loopback origins', () => {
    for (const origin of ['https://example.com', 'null', 'http://127.0.0.1:8080', 'https://127.0.0.1:5173']) {
      expect(admit(request({ host: '127.0.0.1:5173', origin, upgrade: 'websocket' }))).toBe(false)
      expect(admit(request({ host: '127.0.0.1:5173', origin }))).toBe(false)
    }
  })

  it('rejects non-loopback Host headers and an unbound listener', () => {
    expect(admit(request({ host: 'rebind.example:5173', origin: 'http://127.0.0.1:5173' }))).toBe(false)
    expect(devAdmission({ port: () => undefined })(request({ host: '127.0.0.1:5173' }))).toBe(false)
  })

  const edgeHost = 'preview.example.net'
  const alias = 'alias.example.net'
  const edge = devAdmission({ port: () => 5173, edgeHost, edgeAliases: [alias] })
  const edgeHeaders = { host: '127.0.0.1:5173', 'x-forwarded-host': edgeHost, 'x-forwarded-proto': 'https' }

  it('admits explicitly configured edge hosts and aliases, not arbitrary forwarded hosts', () => {
    for (const host of [edgeHost, alias]) {
      expect(edge(request({ ...edgeHeaders, 'x-forwarded-host': host }))).toBe(true)
      expect(edge(request({ ...edgeHeaders, 'x-forwarded-host': host, origin: `https://${host}` }))).toBe(true)
    }
    expect(admit(request(edgeHeaders))).toBe(false)
    for (const host of ['unknown.example', `${edgeHost}:443`, `${edgeHost}, ${alias}`])
      expect(edge(request({ ...edgeHeaders, 'x-forwarded-host': host }))).toBe(false)
    for (const proto of [undefined, 'http', 'https, https'])
      expect(edge(request({ ...edgeHeaders, 'x-forwarded-proto': proto }))).toBe(false)
  })

  it('requires the selected edge Origin, with normalization only for upgrades', () => {
    for (const origin of [undefined, `https://${edgeHost}`, 'http://127.0.0.1:5173', `https://${alias}`, 'null', 'http://127.0.0.1:8080']) {
      expect(edge(request({ ...edgeHeaders, origin }))).toBe(origin === undefined || origin === `https://${edgeHost}`)
      expect(edge(request({ ...edgeHeaders, origin, upgrade: 'websocket' })))
        .toBe(origin === `https://${edgeHost}` || origin === 'http://127.0.0.1:5173')
    }
  })

  it('binds both paths to the actual listener port and rejects before binding', () => {
    for (const host of ['127.0.0.1', '127.0.0.1:8080', 'localhost:8080', 'unknown.example:5173']) {
      expect(admit(request({ host }))).toBe(false)
      expect(edge(request({ ...edgeHeaders, host }))).toBe(false)
    }
    expect(edge(request({ ...edgeHeaders, host: 'localhost:5173' }))).toBe(false)
    let port: number | undefined
    const bound = devAdmission({ port: () => port, edgeHost })
    expect(bound(request(edgeHeaders))).toBe(false)
    port = 5173
    expect(bound(request(edgeHeaders))).toBe(true)
    port = 8080
    expect(bound(request(edgeHeaders))).toBe(false)
  })

  it('rejects duplicated authority headers regardless of case on either path', () => {
    for (const headers of [{ host: '127.0.0.1:5173' }, edgeHeaders]) {
      for (const name of ['host', 'origin', 'sec-fetch-site', 'x-forwarded-host', 'x-forwarded-proto'])
        expect(edge(request(headers, [name, 'first', name.toUpperCase(), 'second']))).toBe(false)
    }
  })

  it('rejects caller credentials and unsupported proxy metadata on either path', () => {
    for (const headers of [{ host: '127.0.0.1:5173' }, edgeHeaders]) {
      for (const name of ['authorization', 'proxy-authorization', 'forwarded', 'x-forwarded-port', 'x-forwarded-server'])
        expect(edge(request({ ...headers, [name]: 'untrusted' }))).toBe(false)
      expect(edge(request({ ...headers, 'x-forwarded-for': '192.0.2.1' }))).toBe(true)
    }
    expect(edge(request({ host: '127.0.0.1:5173', 'x-forwarded-proto': 'https' }))).toBe(false)
  })

  it('requires a loopback peer, origin-form target and safe fetch metadata', () => {
    for (const headers of [{ host: '127.0.0.1:5173' }, edgeHeaders]) {
      for (const peer of ['127.0.0.1', '::1', '::ffff:127.0.0.1'])
        expect(edge(request(headers, [], peer))).toBe(true)
      for (const peer of ['', '192.0.2.1'])
        expect(edge(request(headers, [], peer))).toBe(false)
      for (const url of ['', '//other.example/path', 'https://other.example/path', '*'])
        expect(edge(request(headers, [], '127.0.0.1', url))).toBe(false)
      for (const site of ['cross-site', 'same-site'])
        expect(edge(request({ ...headers, 'sec-fetch-site': site }))).toBe(false)
      for (const site of ['same-origin', 'none'])
        expect(edge(request({ ...headers, 'sec-fetch-site': site }))).toBe(true)
    }
  })
})
