import { expect, it } from 'vitest'
import { checkTagCapabilities } from './tagAuthorization.mts'

const capabilitiesBody = (api_version: string): string => JSON.stringify({
  api_version,
  value: { kind: 'capabilities', capabilities: [{ id: 'read.collections', state: 'granted' }] },
})

it.each(['st3.client.v0', 'fractal.client.v0'])('accepts %s capabilities during credential checks', apiVersion => {
  expect(() => checkTagCapabilities(capabilitiesBody(apiVersion))).not.toThrow()
})

it('rejects an unknown capabilities API version', () => {
  expect(() => checkTagCapabilities(capabilitiesBody('fractal.client.v1'))).toThrow(/api_version/)
})
