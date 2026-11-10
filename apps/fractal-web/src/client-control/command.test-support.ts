import type * as Wire from '@fractal/client/schema'
import { decodeGrant, encodeGrant } from './wire.ts'
import { createClientControlLifecycle } from './lifecycle.ts'

/** Synthetic protocol examples, not golden signing vectors or enrollment authority. */
export const testGrant = (itemId: string = 'attention/unit-ask') => decodeGrant({
  grant_id: 'client-control-grant/unit', run_id: 'mission-run/unit', device: 'device/unit',
  session: 'client-session/unit', actor_agent: 'agent/unit',
  verbs: ['phone.inspect', 'phone.navigate'], screens: ['NowRoot', 'Attention'],
  max_commands: 100, expires_at: '2030-01-01T00:00:00.000Z', state: 'active', item_id: itemId,
})

/** Unknown permits deliberately invalid/refused verbs to exercise the generated decoder. */
export const testCommand = (verb: string, args: unknown = {}, grant: Wire.ClientControlGrant = testGrant()) => ({
  kind: 'device-command', id: 'device-command/unit', revision: 'r1',
  command_kind: verb, args, device_id: grant.device, session_actor: grant.session,
  issued_by: grant.actor_agent, issued_at: '2026-11-01T00:00:00.000Z', updated_at: '2026-11-01T00:00:00.000Z',
  expires_at: '2030-01-01T00:00:00.000Z', state: 'delivered',
  grant_id: grant.grant_id, run_id: grant.run_id,
  control_grant: encodeGrant(grant),
})

export const testLifecycleResource = (
  grant: Wire.ClientControlGrant,
  state: Wire.ClientControlGrantLifecycle['state'] = 'started',
  sequence: number = 1,
) => ({
  kind: 'client-control-grant-lifecycle', id: 'client-control-grant-lifecycle/unit',
  revision: String(sequence), updated_at: '2026-11-10T12:00:00.000Z',
  grant_id: grant.grant_id, run_id: grant.run_id, device: grant.device, session: grant.session,
  ...(grant.item_id === undefined ? {} : { item_id: grant.item_id }),
  state, reason: state, sequence,
})

/** Activates only a synthetic generated lifecycle resource; never application enrollment. */
export const testLifecycle = (grant: Wire.ClientControlGrant = testGrant()) => {
  const lifecycle = createClientControlLifecycle({
    subscriptionId: 'commands',
    authority: grantId => grantId === grant.grant_id ? grant : undefined,
  })
  if (lifecycle.acceptResource(testLifecycleResource(grant)) !== 'activate') {
    throw new Error('Synthetic generated lifecycle start was refused')
  }
  return lifecycle
}
