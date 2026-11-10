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
  readonly currentScreen: () => string
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
  const allowed = () => {
    const permission = input.permission()
    return permission !== undefined && permission.verbs.has(intent.verb)
      && permission.screens.has(destination?._tag ?? input.currentScreen())
  }
  if (!allowed()) return 'Refused'
  if (!input.foreground()) return 'NotForeground'
  if (destination !== undefined) await input.navigate(destination)
  // Navigation may yield; revoked authority or lost foreground must never sign afterward.
  if (!input.permission()?.screens.has(input.currentScreen())) return 'Refused'
  if (!allowed()) return 'Refused'
  if (!input.foreground()) return 'NotForeground'
  await input.signVisibleState()
  return 'Applied'
}
