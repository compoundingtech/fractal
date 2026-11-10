import { describe, expect, it } from 'vitest'
import type * as Wire from '@fractal/client/schema'
import { handleGrantedDeviceCommand } from './grantBoundary.ts'
import type { ClientControlRouteTarget } from './navigation.ts'
import { testCommand, testGrant } from './command.test-support.ts'

const exercise = async (
  command: unknown,
  grant: Wire.ClientControlGrant | undefined = testGrant(),
  initialTarget: ClientControlRouteTarget = { _tag: 'NowRoot' },
) => {
  const navigations: ClientControlRouteTarget[] = []
  let signatures = 0
  let currentTarget = initialTarget
  const result = await handleGrantedDeviceCommand({
    command, permission: () => grant, foreground: () => true,
    currentTarget: () => currentTarget,
    navigate: async target => { navigations.push(target); currentTarget = target },
    signVisibleState: async () => { signatures += 1 },
  })
  return { result, navigations, signatures }
}

const refused = { result: 'Refused', navigations: [], signatures: 0 }
describe('generated client-control grant-check handler boundary', () => {
  it('refuses Conversation under a NowRoot/Attention grant without navigating or signing', async () => {
    expect(await exercise(testCommand('phone.navigate', { screen: 'Conversation', params: { target: 'agent/unit' } }))).toEqual(refused)
  })

  it.each([null, {}, { kind: 'unsupported-event' }])('refuses unsupported event input without fallback', async input => {
    expect(await exercise(input)).toEqual(refused)
  })

  it.each(['completed', 'not-foreground', 'expired', 'refused', 'failed'])('does not replay terminal %s commands', async state => {
    expect(await exercise({ ...testCommand('phone.inspect'), state })).toEqual(refused)
  })

  it.each([
    { screen: 'NowRoot', params: null },
    { screen: 'NowRoot', unexpected: 'value' },
    { screen: 'Attention', params: null },
    { screen: 'Attention' },
    { screen: 'Attention', params: { id: 'attention/unit-ask', unexpected: 'value' } },
  ])('refuses malformed navigation args without normalization: %j', async args => {
    expect(await exercise(testCommand('phone.navigate', args))).toEqual(refused)
  })

  it('refuses an unknown delivered-command key without navigating or signing', async () => {
    expect(await exercise({ ...testCommand('phone.navigate', { screen: 'NowRoot' }), unexpected: 'value' })).toEqual(refused)
  })

  it('accepts absent params only for the generated no-param route', async () => {
    expect(await exercise(testCommand('phone.navigate', { screen: 'NowRoot' }))).toEqual({
      result: 'Applied', navigations: [{ _tag: 'NowRoot' }], signatures: 1,
    })
  })

  it.each(['Agents', 'Missions', 'Usage', 'History'])(
    'refuses nongranted screen %s without navigation or signing', async screen => {
      expect(await exercise(testCommand('phone.navigate', { screen }))).toEqual(refused)
    },
  )

  it.each(['work.done', 'prompt.respond', 'attention.answer', 'attention.close', 'phone.message.read', 'read-position.advance'])(
    'refuses %s without navigation or a signed result', async verb => {
      const grant = testGrant()
      const args = verb === 'phone.message.read' ? { message_id: 'message/unit' } : {}
      expect(await exercise(testCommand(verb, args), { ...grant, verbs: [...grant.verbs, verb] })).toEqual(refused)
    },
  )

  it('only exposes local routing and inspect/sign, never an answer-control activation capability', async () => {
    expect(await exercise(testCommand('phone.navigate', { screen: 'Attention', params: { id: 'attention/unit-ask' } }))).toEqual({
      result: 'Applied', navigations: [{ _tag: 'Attention', askId: 'attention/unit-ask' }], signatures: 1,
    })
    expect(await exercise(testCommand('phone.inspect'))).toEqual({ result: 'Applied', navigations: [], signatures: 1 })
  })

  it('refuses a grant that omits the destination', async () => {
    expect(await exercise(testCommand('phone.navigate', { screen: 'Attention', params: { id: 'attention/unit-ask' } }),
      { ...testGrant(), screens: ['NowRoot'] })).toEqual(refused)
  })

  it('refuses navigation to an ask other than the grant-bound item', async () => {
    expect(await exercise(testCommand('phone.navigate', { screen: 'Attention', params: { id: 'attention/other-ask' } }))).toEqual(refused)
  })

  it('refuses inspecting an ask other than the grant-bound item', async () => {
    expect(await exercise(testCommand('phone.inspect'), testGrant(), { _tag: 'Attention', askId: 'attention/other-ask' })).toEqual(refused)
  })

  it('allows optional item_id absence for NowRoot but refuses unbound Attention', async () => {
    const { item_id: _item, ...unbound } = testGrant()
    expect(await exercise(testCommand('phone.inspect'), unbound)).toEqual({ result: 'Applied', navigations: [], signatures: 1 })
    expect(await exercise(testCommand('phone.navigate', { screen: 'Attention', params: { id: 'attention/unit-ask' } }), unbound)).toEqual(refused)
  })

  it.each(['device_id', 'grant_id', 'run_id'] as const)('refuses mismatched %s binding', async field => {
    expect(await exercise({ ...testCommand('phone.inspect'), [field]: 'other/unit' })).toEqual(refused)
  })

  it.each([
    [undefined, true, 'Refused'],
    [testGrant(), false, 'NotForeground'],
    [{ ...testGrant(), verbs: [] }, true, 'Refused'],
  ] as const)('does not act without permission or foreground', async (grant, foreground, expected) => {
    const calls: string[] = []
    const result = await handleGrantedDeviceCommand({
      command: testCommand('phone.navigate', { screen: 'NowRoot' }), permission: () => grant,
      foreground: () => foreground, currentTarget: () => ({ _tag: 'NowRoot' }),
      navigate: async () => { calls.push('navigate') }, signVisibleState: async () => { calls.push('sign') },
    })
    expect(result).toBe(expected)
    expect(calls).toEqual([])
  })

  it('does not sign after grant loss during navigation', async () => {
    let active: Wire.ClientControlGrant | undefined = testGrant()
    let signatures = 0
    const result = await handleGrantedDeviceCommand({
      command: testCommand('phone.navigate', { screen: 'NowRoot' }), permission: () => active, foreground: () => true,
      currentTarget: () => ({ _tag: 'NowRoot' }),
      navigate: async () => { active = undefined }, signVisibleState: async () => { signatures += 1 },
    })
    expect(result).toBe('Refused')
    expect(signatures).toBe(0)
  })
})

// Authoritative enrollment/session/window admission remains the later daemon adapter's boundary.
