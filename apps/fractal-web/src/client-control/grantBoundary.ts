import type { ClientControlRouteTarget } from './navigation.ts'

/** Internal adapter inputs, NOT copied wire types. Decode generated commands before this boundary. */
export interface ClientControlIntent {
  readonly verb: string
  readonly screenName?: string
  readonly targetAskId?: string
}
export interface ClientControlPermission {
  /** Authoritative adapter returns nothing unless identity/session/run/grant binding is valid. */
  readonly verbs: ReadonlySet<string>
  readonly screens: ReadonlySet<string>
  /** Internal authority projection of the grant's bound item, not a wire field name. */
  readonly boundAskId: string
}

const destinationFor = (intent: ClientControlIntent): ClientControlRouteTarget | undefined => {
  if (intent.screenName === 'NowRoot' && intent.targetAskId === undefined) return { _tag: 'NowRoot' }
  if (intent.screenName === 'Attention' && intent.targetAskId !== undefined && intent.targetAskId !== '')
    return { _tag: 'Attention', askId: intent.targetAskId }
  return undefined
}

/**
 * Narrow handler boundary: there is deliberately no answer/close/read-state callback or arbitrary
 * DOM activation capability. Only granted inspect and the final NowRoot/Attention route set exist.
 * The later generated-wire adapter owns identity/session/run/window validation and permission revocation.
 */
export const handleGrantedClientControlIntent = async (input: {
  readonly currentTarget: () => ClientControlRouteTarget | undefined
  readonly intent: ClientControlIntent
  readonly permission: () => ClientControlPermission | undefined
  readonly foreground: () => boolean
  readonly navigate: (target: ClientControlRouteTarget) => Promise<void>
  readonly signVisibleState: () => Promise<void>
}): Promise<'Refused' | 'NotForeground' | 'Applied'> => {
  const intent = input.intent
  if (intent.verb !== 'phone.inspect' && intent.verb !== 'phone.navigate') return 'Refused'
  const destination = intent.verb === 'phone.navigate' ? destinationFor(intent) : undefined
  if (intent.verb === 'phone.navigate' && destination === undefined) return 'Refused'
  if (intent.verb === 'phone.inspect' && (intent.screenName !== undefined || intent.targetAskId !== undefined)) return 'Refused'
  const targetAllowed = (permission: ClientControlPermission, target: ClientControlRouteTarget | undefined) =>
    target !== undefined && permission.screens.has(target._tag)
      && (target._tag !== 'Attention' || target.askId === permission.boundAskId)
  const allowed = () => {
    const permission = input.permission()
    return permission !== undefined && permission.verbs.has(intent.verb)
      && targetAllowed(permission, destination ?? input.currentTarget())
  }
  if (!allowed()) return 'Refused'
  if (!input.foreground()) return 'NotForeground'
  if (destination !== undefined) await input.navigate(destination)
  // Navigation may yield; revoked authority or lost foreground must never sign afterward.
  const permission = input.permission()
  if (permission === undefined || !targetAllowed(permission, input.currentTarget())) return 'Refused'
  if (!allowed()) return 'Refused'
  if (!input.foreground()) return 'NotForeground'
  await input.signVisibleState()
  return 'Applied'
}
