// @vitest-environment node
import { IDBFactory } from 'fake-indexeddb'
import { afterEach, describe, expect, it, vi } from 'vitest'
import golden from '../../../../fixtures/clients/client-control-v1.golden.json' with { type: 'json' }
import {
  CLIENT_RESULT_SIGNED_FIELDS,
  clientResultCanonicalBytes,
  visibleStateCanonicalJson,
  visibleStateDigest,
} from '../../../../clients/typescript/fractal-client/ClientControl.generated.ts'
import {
  decodeBinding, encodeBinding, decodeCommand, encodeCommand, decodeGrant, encodeGrant,
  decodeResult, encodeResult, decodeReceipt, encodeReceipt, signDeviceResult, verifyClientResultBinding,
} from './wire.ts'
import { getOrCreateClientControlIdentity } from './identity.ts'
import { testCommand, testGrant } from './command.test-support.ts'

const bytes = (value: string): Uint8Array<ArrayBuffer> => Uint8Array.from(Buffer.from(value, 'base64url'))
const verificationKey = () => crypto.subtle.importKey(
  'raw', bytes(golden.public_key_sec1_base64url), { name: 'ECDSA', namedCurve: 'P-256' }, true, ['verify'],
)
afterEach(() => vi.unstubAllGlobals())

describe('web consumption of the repository client-control golden file', () => {
  it.each(golden.vectors)('decodes/encodes and verifies every shared vector: $name', async vector => {
    const binding = encodeBinding(decodeBinding(vector.input))
    expect(binding).toEqual(vector.input)
    expect(visibleStateCanonicalJson(vector.input.visible_state)).toBe(vector.canonical_visible_state)
    expect(await visibleStateDigest(vector.input.visible_state)).toBe(vector.visible_state_sha256)
    expect(new TextDecoder().decode(await clientResultCanonicalBytes(vector.input))).toBe(vector.canonical_bytes)
    const key = await verificationKey()
    expect(await verifyClientResultBinding(binding, vector.signature_p1363_base64url, key)).toBe(true)
    for (const changed of vector.negative_bindings) {
      expect(await verifyClientResultBinding(changed, vector.signature_p1363_base64url, key)).toBe(false)
      // Even negatives excluded by the strict wire schema must fail cryptographic verification.
      expect(await crypto.subtle.verify(
        { name: 'ECDSA', hash: 'SHA-256' }, key, bytes(vector.signature_p1363_base64url),
        await clientResultCanonicalBytes(changed) as Uint8Array<ArrayBuffer>,
      )).toBe(false)
    }
    expect(await verifyClientResultBinding(binding, golden.message_domain_signature_p1363_base64url, key)).toBe(false)
    expect(await crypto.subtle.verify(
      { name: 'ECDSA', hash: 'SHA-256' }, key, bytes(vector.signature_p1363_base64url),
      new TextEncoder().encode(golden.message_domain_bytes),
    )).toBe(false)
  })

  it('verifies the shared message signature only in its own domain', async () => {
    expect(await crypto.subtle.verify(
      { name: 'ECDSA', hash: 'SHA-256' }, await verificationKey(),
      bytes(golden.message_domain_signature_p1363_base64url), new TextEncoder().encode(golden.message_domain_bytes),
    )).toBe(true)
  })

  it.each(golden.vectors)('signs and encodes a real web identity result using shared binding $name', async vector => {
    vi.stubGlobal('indexedDB', new IDBFactory())
    const identity = await getOrCreateClientControlIdentity()
    const key = await crypto.subtle.importKey('spki', await identity.exportPublicKey(), { name: 'ECDSA', namedCurve: 'P-256' }, true, ['verify'])
    const result = await signDeviceResult({
      binding: decodeBinding(vector.input), identity,
      signatureMetadata: { chain: [], format: 'fields-v1', key: 'public-test-key', nonce: 'unit-result-nonce-0001', signed_at_unix_ms: 0, signer: 'person/unit' },
    })
    expect(encodeResult(decodeResult(result))).toEqual(result)
    expect(result.binding).toEqual(vector.input)
    expect(result.result).toEqual(vector.input.visible_state)
    expect(result.command_id).toBe(vector.input.command_id)
    expect(result.outcome).toBe(vector.input.outcome)
    expect(result.signature!.signed_fields).toEqual(CLIENT_RESULT_SIGNED_FIELDS)
    expect(bytes(result.signature!.signature)).toHaveLength(64)
    expect(result.signature!.signature).toMatch(/^[A-Za-z0-9_-]+$/u)
    expect(await verifyClientResultBinding(result.binding, result.signature!.signature, key)).toBe(true)
  })

  it('roundtrips generated grants with optional item_id and delivered native Attention commands', () => {
    const bound = encodeGrant(testGrant())
    expect(encodeGrant(decodeGrant(bound))).toEqual(bound)
    const { item_id: _item, ...unbound } = bound
    expect(encodeGrant(decodeGrant(unbound))).toEqual(unbound)
    for (const control_grant of [bound, unbound]) {
      const command = { ...testCommand('phone.navigate', { screen: 'Attention', params: { id: bound.item_id! } }), control_grant }
      expect(encodeCommand(decodeCommand(command))).toEqual(command)
    }
  })

  it('uses the generated durable-receipt evidence schema without defining a local wire shape', () => {
    const vector = golden.vectors[0]!
    const receipt = {
      commandId: vector.input.command_id, commandKind: vector.input.command_kind,
      device: vector.input.device, session: vector.input.session, grantId: vector.input.grant_id,
      runId: vector.input.run_id, issuer: 'agent/unit', verifiedResultSignature: null,
      signedResult: vector.input, terminalState: vector.input.outcome,
    }
    expect(encodeReceipt(decodeReceipt(receipt))).toEqual(receipt)
  })
})

// The shared file has two signing vectors and twenty negative bindings. It has no grant,
// command, envelope/receipt or fingerprint vectors: those roundtrips are schema examples,
// not golden-byte conformance. No web-relevant vector in the published file is skipped.
