// @vitest-environment node
import { IDBFactory } from 'fake-indexeddb'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  ClientControlIdentityError,
  generateClientControlKeyPair,
  getOrCreateClientControlIdentity,
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
    // No signature length/serialization assertion: daemon canonical encoding is unresolved.
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
