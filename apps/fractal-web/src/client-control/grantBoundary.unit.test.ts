import { describe, expect, it } from 'vitest'
import { handleGrantedClientControlIntent, type ClientControlIntent, type ClientControlPermission } from './grantBoundary.ts'
import type { ClientControlRouteTarget } from './navigation.ts'

const permission: ClientControlPermission = {
  verbs: new Set(['phone.inspect', 'phone.navigate']), screens: new Set(['NowRoot', 'Attention']),
}

const exercise = async (intent: ClientControlIntent, grant: ClientControlPermission | undefined = permission) => {
  const navigations: ClientControlRouteTarget[] = []
  let signatures = 0
  const result = await handleGrantedClientControlIntent({
    intent, permission: () => grant, foreground: () => true,
    currentScreen: () => intent.screenName === 'Attention' ? 'Attention' : 'NowRoot',
    navigate: async target => { navigations.push(target) },
    signVisibleState: async () => { signatures += 1 },
  })
  return { result, navigations, signatures }
}

describe('client-control grant-check handler boundary', () => {
  it('refuses Conversation under a NowRoot/Attention grant without navigating or signing', async () => {
    expect(await exercise({ verb: 'phone.navigate', screenName: 'Conversation' })).toEqual({
      result: 'Refused', navigations: [], signatures: 0,
    })
  })

  it.each(['work.done', 'prompt.respond', 'attention.answer', 'attention.close', 'phone.message.read', 'read-position.advance'])(
    'refuses %s without navigation or a signed result', async verb => {
      // Even a malformed over-broad grant cannot turn the narrow handler into a write executor.
      const grant = { ...permission, verbs: new Set([...permission.verbs, verb]) }
      expect(await exercise({ verb }, grant)).toEqual({ result: 'Refused', navigations: [], signatures: 0 })
    },
  )

  it('only exposes local routing and inspect/sign, never an answer-control activation capability', async () => {
    expect(await exercise({ verb: 'phone.navigate', screenName: 'Attention', targetAskId: 'attention/unit-ask' })).toEqual({
      result: 'Applied', navigations: [{ _tag: 'Attention', askId: 'attention/unit-ask' }], signatures: 1,
    })
    expect(await exercise({ verb: 'phone.inspect' })).toEqual({ result: 'Applied', navigations: [], signatures: 1 })
  })

  it('refuses a grant that omits the destination', async () => {
    expect(await exercise({ verb: 'phone.navigate', screenName: 'Attention', targetAskId: 'attention/unit-ask' },
      { ...permission, screens: new Set(['NowRoot']) })).toEqual({ result: 'Refused', navigations: [], signatures: 0 })
  })

  it.each([
    [undefined, true, 'Refused'],
    [permission, false, 'NotForeground'],
    [{ ...permission, verbs: new Set<string>() }, true, 'Refused'],
  ] as const)('does not act without permission or foreground', async (grant, foreground, expected) => {
    const calls: string[] = []
    const result = await handleGrantedClientControlIntent({
      intent: { verb: 'phone.navigate', screenName: 'NowRoot' }, permission: () => grant,
      foreground: () => foreground, currentScreen: () => 'NowRoot',
      navigate: async () => { calls.push('navigate') }, signVisibleState: async () => { calls.push('sign') },
    })
    expect(result).toBe(expected)
    expect(calls).toEqual([])
  })

  it('does not sign after grant loss during navigation', async () => {
    let active: ClientControlPermission | undefined = permission
    let signatures = 0
    const result = await handleGrantedClientControlIntent({
      intent: { verb: 'phone.navigate', screenName: 'NowRoot' }, permission: () => active, foreground: () => true,
      currentScreen: () => 'NowRoot',
      navigate: async () => { active = undefined }, signVisibleState: async () => { signatures += 1 },
    })
    expect(result).toBe('Refused')
    expect(signatures).toBe(0)
  })
})

// These assertions run against the wire-independent handler. Generated command decoding,
// identity/session/run/window authority and canonical signing conformance remain pending #6274.
