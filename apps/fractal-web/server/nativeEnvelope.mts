import { PairingAdvertisement } from '@fractal/client/schema/read'
import { Schema } from 'effect'

// The generated native API version schema owns receiver compatibility, not sender defaults.
export const NativeApiVersion = PairingAdvertisement.fields.api_version
export const admittedNativeApiVersions = NativeApiVersion.literals
export const nativeEnvelope = <S extends Schema.Top>(value: S) => Schema.Struct({ api_version: NativeApiVersion, value })
