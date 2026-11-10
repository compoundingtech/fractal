import { decodeGrant } from './wire.ts'

/** Synthetic protocol examples, not golden signing vectors or enrollment authority. */
export const testGrant = (itemId: string = 'attention/unit-ask') => decodeGrant({
  grant_id: 'client-control-grant/unit', run_id: 'mission-run/unit', device: 'device/unit',
  session: 'client-session/unit', actor_agent: 'agent/unit',
  verbs: ['phone.inspect', 'phone.navigate'], screens: ['NowRoot', 'Attention'],
  max_commands: 100, expires_at: '2030-01-01T00:00:00.000Z', state: 'active', item_id: itemId,
})

/** Unknown permits deliberately invalid/refused verbs to exercise the generated decoder. */
export const testCommand = (verb: string, args: unknown = {}) => ({
  kind: 'device-command', id: 'device-command/unit', revision: 'r1',
  command_kind: verb, args, device_id: 'device/unit', session_actor: 'client-session/unit',
  issued_by: 'agent/unit', issued_at: '2026-11-01T00:00:00.000Z', updated_at: '2026-11-01T00:00:00.000Z',
  expires_at: '2030-01-01T00:00:00.000Z', state: 'delivered',
  grant_id: 'client-control-grant/unit', run_id: 'mission-run/unit',
})
