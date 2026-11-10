// @vitest-environment node
import { IDBFactory } from 'fake-indexeddb'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  ClientControlIdentityError,
  generateClientControlKeyPair,
  getOrCreateClientControlIdentity,
  readExistingPublicKeyPoint,
} from './identity.ts'

// Crypto is the runtime's REAL WebCrypto, never a mock. Only IndexedDB uses the
// in-memory platform implementation; these are not browser persistence receipts.
const subtle = globalThis.crypto.subtle
const bytes = new TextEncoder().encode('caller-supplied canonical bytes; not a device.result contract')
const algorithm = { name: 'ECDSA', hash: 'SHA-256' }

beforeEach(() => { vi.stubGlobal('indexedDB', new IDBFactory()) })
afterEach(() => { vi.unstubAllGlobals() })

describe('client-control P-256 identity', () => {
  it('generates a non-extractable signing key and an exportable verification key', async () => {
    const keys = await generateClientControlKeyPair()
    expect(keys.privateKey.type).toBe('private')
    expect(keys.privateKey.extractable).toBe(false)
    expect(keys.privateKey.algorithm).toMatchObject({ name: 'ECDSA', namedCurve: 'P-256' })
    expect(keys.privateKey.usages).toEqual(['sign'])
    await expect(subtle.exportKey('pkcs8', keys.privateKey)).rejects.toThrow()
    await expect(subtle.exportKey('jwk', keys.privateKey)).rejects.toThrow()
    expect(keys.publicKey.extractable).toBe(true)
    const exported = await subtle.exportKey('spki', keys.publicKey)
    const imported = await subtle.importKey('spki', exported, { name: 'ECDSA', namedCurve: 'P-256' }, true, ['verify'])
    const signature = await subtle.sign(algorithm, keys.privateKey, bytes)
    expect(await subtle.verify(algorithm, imported, signature, bytes)).toBe(true)
  })

  it('exports its public key and signs exactly the supplied bytes', async () => {
    const identity = await getOrCreateClientControlIdentity()
    const key = await subtle.importKey('spki', await identity.exportPublicKey(), { name: 'ECDSA', namedCurve: 'P-256' }, true, ['verify'])
    const signature = await identity.signCanonicalBytes(bytes)
    expect(await subtle.verify(algorithm, key, signature, bytes)).toBe(true)
    expect(await subtle.verify(algorithm, key, signature, new TextEncoder().encode('different bytes'))).toBe(false)
    const other = await generateClientControlKeyPair()
    expect(await subtle.verify(algorithm, other.publicKey, signature, bytes)).toBe(false)
    // Generated canonical signing/envelope conformance belongs to wire.unit.test.ts.
  })

  it('retains the committed identity across independent acquisitions without a key cache', async () => {
    const initial = await getOrCreateClientControlIdentity()
    const publicKey = await initial.exportPublicKey()
    const reloaded = await getOrCreateClientControlIdentity()
    expect(new Uint8Array(await reloaded.exportPublicKey())).toEqual(new Uint8Array(publicKey))
    const verifier = await subtle.importKey('spki', publicKey, { name: 'ECDSA', namedCurve: 'P-256' }, true, ['verify'])
    expect(await subtle.verify(algorithm, verifier, await reloaded.signCanonicalBytes(bytes), bytes)).toBe(true)
  })

  it('returns one committed key to concurrent first acquisitions', async () => {
    const identities = await Promise.all(Array.from({ length: 8 }, () => getOrCreateClientControlIdentity()))
    const publicKeys = await Promise.all(identities.map(identity => identity.exportPublicKey()))
    for (const publicKey of publicKeys) {
      expect(new Uint8Array(publicKey)).toEqual(new Uint8Array(publicKeys[0]!))
    }
    const verifier = await subtle.importKey('spki', publicKeys[0]!, { name: 'ECDSA', namedCurve: 'P-256' }, true, ['verify'])
    for (const identity of identities) {
      expect(await subtle.verify(algorithm, verifier, await identity.signCanonicalBytes(bytes), bytes)).toBe(true)
    }
    const later = await getOrCreateClientControlIdentity()
    expect(new Uint8Array(await later.exportPublicKey())).toEqual(new Uint8Array(publicKeys[0]!))
  })

  it('reads absent identity without creating a database', async () => {
    expect(await indexedDB.databases()).toEqual([])
    expect(await readExistingPublicKeyPoint()).toEqual({ _tag: 'Absent' })
    expect(await indexedDB.databases()).toEqual([])
  })

  it('reads the existing public SEC1 point without exporting a private key', async () => {
    const identity = await getOrCreateClientControlIdentity()
    const spki = await identity.exportPublicKey()
    const publicKey = await subtle.importKey('spki', spki, { name: 'ECDSA', namedCurve: 'P-256' }, true, ['verify'])
    const expected = new Uint8Array(await subtle.exportKey('raw', publicKey))
    const exports = vi.spyOn(subtle, 'exportKey')
    try {
      const result = await readExistingPublicKeyPoint()
      expect(result._tag).toBe('Present')
      if (result._tag !== 'Present') throw new Error('Existing identity missing')
      expect(result.point).toEqual(expected)
      expect(result.point.length).toBe(65)
      expect(result.point[0]).toBe(0x04)
      expect(exports).toHaveBeenCalledTimes(1)
      expect(exports.mock.calls[0]![0]).toBe('spki')
      expect(exports.mock.calls[0]![1].type).toBe('public')
    } finally {
      exports.mockRestore()
    }
    expect(new Uint8Array(await identity.exportPublicKey())).toEqual(new Uint8Array(spki))
  })

  it('fails closed without IndexedDB instead of creating an ephemeral identity', async () => {
    vi.stubGlobal('indexedDB', undefined)
    await expect(getOrCreateClientControlIdentity()).rejects.toMatchObject({
      _tag: 'ClientControlIdentityError', code: 'unavailable',
    })
  })

  it('fails closed without secure-context WebCrypto', async () => {
    vi.stubGlobal('crypto', undefined)
    await expect(getOrCreateClientControlIdentity()).rejects.toBeInstanceOf(ClientControlIdentityError)
    await expect(getOrCreateClientControlIdentity()).rejects.toMatchObject({ code: 'unavailable' })
  })
})
