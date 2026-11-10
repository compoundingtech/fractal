// This module is an internal capability, never a window/debug/control API. Obtaining a key
// does not enable client control or establish pairing/enrollment/grant authority.
const databaseName = 'fractal-client-control-identity-v1'
const storeName = 'identity'
const identityKey = 'p256'

export type ClientControlIdentityErrorCode =
  | 'unavailable'
  | 'storage-blocked'
  | 'storage-failed'
  | 'invalid-stored-identity'
  | 'generation-failed'
  | 'export-failed'
  | 'signing-failed'

export class ClientControlIdentityError extends Error {
  readonly _tag = 'ClientControlIdentityError'

  constructor(readonly code: ClientControlIdentityErrorCode, message: string, cause?: unknown) {
    super(message, { cause })
    this.name = 'ClientControlIdentityError'
  }
}

export interface ClientControlIdentity {
  /** DER SubjectPublicKeyInfo bytes; pairing's eventual representation is not decided here. */
  exportPublicKey(): Promise<ArrayBuffer>
  /**
   * Signs caller-supplied canonical bytes, without encoding or constructing a device.result.
   * The caller must enforce enrollment, foreground, command and active-grant checks first.
   * Returns WebCrypto's native signature bytes; no daemon serialization is assumed.
   */
  signCanonicalBytes(bytes: Uint8Array<ArrayBuffer>): Promise<ArrayBuffer>
}

/** Low-level key generation used by persistence and real-WebCrypto tests; never global. */
export const generateClientControlKeyPair = async (subtle: SubtleCrypto = globalThis.crypto.subtle): Promise<CryptoKeyPair> => {
  try {
    // WebCrypto keeps the public half exportable while making the private half non-extractable.
    return await subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign', 'verify'])
  } catch (cause) {
    throw new ClientControlIdentityError('generation-failed', 'Could not generate the client-control identity', cause)
  }
}

type StoredIdentity = { version: 1; privateKey: CryptoKey; publicKey: CryptoKey }

const isP256Key = (key: unknown): key is CryptoKey =>
  key instanceof CryptoKey && key.algorithm.name === 'ECDSA'
  && 'namedCurve' in key.algorithm && key.algorithm.namedCurve === 'P-256'

const isStoredIdentity = (value: unknown): value is StoredIdentity => {
  if (typeof value !== 'object' || value === null || !('version' in value) || value.version !== 1
    || !('privateKey' in value) || !('publicKey' in value)) return false
  return isP256Key(value.privateKey) && value.privateKey.type === 'private'
    && !value.privateKey.extractable && value.privateKey.usages.length === 1 && value.privateKey.usages[0] === 'sign'
    && isP256Key(value.publicKey) && value.publicKey.type === 'public'
    && value.publicKey.extractable && value.publicKey.usages.length === 1 && value.publicKey.usages[0] === 'verify'
}

const openDatabase = (factory: IDBFactory): Promise<IDBDatabase> => {
  const { promise, resolve, reject } = Promise.withResolvers<IDBDatabase>()
  const request = factory.open(databaseName, 1)
  let rejected = false
  request.onupgradeneeded = () => { request.result.createObjectStore(storeName) }
  request.onblocked = () => {
    rejected = true
    reject(new ClientControlIdentityError('storage-blocked', 'Client-control identity storage is blocked by another tab'))
  }
  request.onerror = () => reject(new ClientControlIdentityError('storage-failed', 'Could not open client-control identity storage', request.error))
  request.onsuccess = () => {
    const database = request.result
    if (rejected) { database.close(); return }
    database.onversionchange = () => database.close()
    resolve(database)
  }
  return promise
}

// The read and conditional add share ONE readwrite transaction. IndexedDB serializes these
// transactions across connections/tabs; losing candidates are discarded, never replacing a key.
// Generate outside the transaction: awaiting WebCrypto inside it would let it auto-commit.
const readOrInsert = (database: IDBDatabase, candidate?: CryptoKeyPair): Promise<StoredIdentity | undefined> => {
  const { promise, resolve, reject } = Promise.withResolvers<StoredIdentity | undefined>()
  const transaction = database.transaction(storeName, candidate === undefined ? 'readonly' : 'readwrite')
  const store = transaction.objectStore(storeName)
  const request = store.get(identityKey)
  let identity: StoredIdentity | undefined
  let failure: ClientControlIdentityError | undefined
  transaction.oncomplete = () => resolve(identity)
  transaction.onabort = () => reject(failure ?? new ClientControlIdentityError('storage-failed', 'Could not persist the client-control identity', transaction.error))
  request.onsuccess = () => {
    const value: unknown = request.result
    if (value !== undefined) {
      if (!isStoredIdentity(value)) {
        failure = new ClientControlIdentityError('invalid-stored-identity', 'Stored client-control identity is invalid; refusing to replace it')
        transaction.abort()
        return
      }
      identity = value
    } else if (candidate !== undefined) {
      identity = { version: 1, privateKey: candidate.privateKey, publicKey: candidate.publicKey }
      try {
        store.add(identity, identityKey)
      } catch (cause) {
        failure = new ClientControlIdentityError('storage-failed', 'Could not store the non-extractable client-control key', cause)
        transaction.abort()
      }
    }
  }
  return promise
}

/**
 * Retrieves or atomically creates this origin's identity. Reloads and concurrent tabs use
 * the committed key, not a tab-local cache. Each call closes its database connection.
 * Storage failures never fall back to an ephemeral key or silently rotate an existing key.
 * Clearing site data loses the key and requires new pairing/enrollment; revoke is NOT key deletion.
 * Canonical signed fields/bytes and pairing integration remain the daemon contract's responsibility.
 */
export const getOrCreateClientControlIdentity = async (): Promise<ClientControlIdentity> => {
  if (globalThis.crypto?.subtle === undefined || globalThis.indexedDB === undefined) {
    throw new ClientControlIdentityError('unavailable', 'Client-control identity requires secure-context WebCrypto and IndexedDB')
  }
  const subtle = globalThis.crypto.subtle
  let database: IDBDatabase | undefined
  try {
    database = await openDatabase(globalThis.indexedDB)
    const stored = await readOrInsert(database)
      ?? await readOrInsert(database, await generateClientControlKeyPair(subtle))
    if (stored === undefined) {
      throw new ClientControlIdentityError('storage-failed', 'Client-control identity transaction committed without a key')
    }
    return Object.freeze({
      exportPublicKey: async (): Promise<ArrayBuffer> => {
        try { return await subtle.exportKey('spki', stored.publicKey) }
        catch (cause) { throw new ClientControlIdentityError('export-failed', 'Could not export the client-control public key', cause) }
      },
      signCanonicalBytes: async (bytes: Uint8Array<ArrayBuffer>): Promise<ArrayBuffer> => {
        try { return await subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, stored.privateKey, bytes) }
        catch (cause) { throw new ClientControlIdentityError('signing-failed', 'Could not sign the supplied canonical bytes', cause) }
      },
    })
  } catch (cause) {
    if (cause instanceof ClientControlIdentityError) throw cause
    throw new ClientControlIdentityError('storage-failed', 'Could not access client-control identity storage', cause)
  } finally {
    database?.close()
  }
}
