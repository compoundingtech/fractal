import type * as Wire from '@fractal/client/schema'
import type { ClientControlRouteTarget } from './navigation.ts'
import { decodeCommand } from './wire.ts'

const destinationFor = (command: Wire.DeviceCommand): ClientControlRouteTarget | undefined => {
  if (command.command_kind !== 'phone.navigate') return undefined
  if (command.args.screen === 'NowRoot') return { _tag: 'NowRoot' }
  if (command.args.screen === 'Attention') return { _tag: 'Attention', askId: command.args.params.id }
  return undefined
}

/**
 * Decodes the actual generated delivered command. No answer/close/read-state or DOM activation
 * callback exists. The authority adapter must return an active enrollment-validated grant only;
 * decoding a grant does not establish enrollment, foreground, expiry or session authority.
 */
export const handleGrantedDeviceCommand = async (input: {
  readonly currentTarget: () => ClientControlRouteTarget | undefined
  readonly command: unknown
  readonly permission: () => Wire.ClientControlGrant | undefined
  readonly foreground: () => boolean
  readonly navigate: (target: ClientControlRouteTarget) => Promise<void>
  readonly signVisibleState: () => Promise<void>
}): Promise<'Refused' | 'NotForeground' | 'Applied'> => {
  let command: Wire.DeviceCommand
  try { command = decodeCommand(input.command) } catch { return 'Refused' }
  if (command.state !== 'pending' && command.state !== 'delivered') return 'Refused'
  if (command.command_kind !== 'phone.inspect' && command.command_kind !== 'phone.navigate') return 'Refused'
  const destination = destinationFor(command)
  if (command.command_kind === 'phone.navigate' && destination === undefined) return 'Refused'
  const targetAllowed = (permission: Wire.ClientControlGrant, target: ClientControlRouteTarget | undefined) =>
    target !== undefined && permission.screens.includes(target._tag)
      && (target._tag !== 'Attention' || (permission.item_id !== undefined && target.askId === permission.item_id))
  const allowed = () => {
    const permission = input.permission()
    return permission !== undefined && permission.verbs.includes(command.command_kind)
      && command.device_id === permission.device && command.grant_id === permission.grant_id
      && command.run_id === permission.run_id
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
