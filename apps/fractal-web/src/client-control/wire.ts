import { Schema } from 'effect'
import * as Wire from '@fractal/client/schema'
import {
  CLIENT_RESULT_SIGNED_FIELDS,
  clientResultCanonicalBytes,
  type ClientResultBinding as CanonicalBinding,
} from '../../../../clients/typescript/fractal-client/ClientControl.generated.ts'
import type { ClientControlIdentity } from './identity.ts'

export const decodeCommand = Wire.decodeUnknownSync(Wire.DeviceCommand, 'strict')
export const encodeCommand = Schema.encodeSync(Wire.DeviceCommand)
export const decodeGrant = Wire.decodeUnknownSync(Wire.ClientControlGrant, 'strict')
export const encodeGrant = Schema.encodeSync(Wire.ClientControlGrant)
export const decodeBinding = Wire.decodeUnknownSync(Wire.ClientResultBinding, 'strict')
export const encodeBinding = Schema.encodeSync(Wire.ClientResultBinding)
export const decodeResult = Wire.decodeUnknownSync(Wire.DeviceResult, 'strict')
export const encodeResult = Schema.encodeSync(Wire.DeviceResult)
export const decodeReceipt = Wire.decodeUnknownSync(Wire.ClientCommandReceipt, 'strict')
export const encodeReceipt = Schema.encodeSync(Wire.ClientCommandReceipt)

const base64url = (bytes: Uint8Array): string =>
  btoa(Array.from(bytes, byte => String.fromCharCode(byte)).join('')).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/u, '')
const fromBase64url = (value: string): Uint8Array<ArrayBuffer> | undefined => {
  // A 64-byte P1363 signature has exactly 86 unpadded base64url characters.
  if (!/^[A-Za-z0-9_-]{86}$/u.test(value)) return undefined
  let bytes: Uint8Array<ArrayBuffer>
  try {
    bytes = Uint8Array.from(atob(value.replaceAll('-', '+').replaceAll('_', '/')), byte => byte.charCodeAt(0))
  } catch (error) {
    if (error instanceof DOMException && error.name === 'InvalidCharacterError') return undefined
    throw error
  }
  return bytes.length === 64 && base64url(bytes) === value ? bytes : undefined
}

/** Authority/foreground checks belong to the command handler; this codec creates no authority. */
export const signDeviceResult = async (input: {
  readonly binding: Wire.ClientResultBinding
  readonly signatureMetadata: Omit<Wire.DeviceSignatureEncoded, 'signature' | 'signed_fields'>
  readonly identity: ClientControlIdentity
}): Promise<Wire.DeviceResultEncoded> => {
  const binding = encodeBinding(input.binding)
  // The generated helper constructs these bytes with TextEncoder (ArrayBuffer-backed).
  const bytes = await clientResultCanonicalBytes(binding as CanonicalBinding) as Uint8Array<ArrayBuffer>
  const signature = await input.identity.signCanonicalBytes(bytes)
  return encodeResult(decodeResult({
    command_id: binding.command_id, outcome: binding.outcome,
    ...(binding.error_code === null ? {} : { error_code: binding.error_code }),
    binding, result: binding.visible_state,
    signature: { ...input.signatureMetadata, signature: base64url(new Uint8Array(signature)), signed_fields: [...CLIENT_RESULT_SIGNED_FIELDS] },
  }))
}

/** Invalid binding/schema or signature is a verification refusal, not an alternate encoding. */
export const verifyClientResultBinding = async (binding: unknown, signature: string, key: CryptoKey): Promise<boolean> => {
  let decoded: Wire.ClientResultBinding
  try { decoded = decodeBinding(binding) } catch { return false }
  const signatureBytes = fromBase64url(signature)
  if (signatureBytes === undefined) return false
  const bytes = await clientResultCanonicalBytes(encodeBinding(decoded) as CanonicalBinding) as Uint8Array<ArrayBuffer>
  try {
    return await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, key, signatureBytes, bytes)
  } catch (error) {
    // InvalidAccessError (wrong key/algorithm), TypeError and other programmer errors propagate.
    if (error instanceof DOMException && (error.name === 'DataError' || error.name === 'OperationError')) return false
    throw error
  }
}
