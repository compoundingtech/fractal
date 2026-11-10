import * as Wire from '@fractal/client/schema'

const decodeLifecycle = Wire.decodeUnknownSync(Wire.ClientControlGrantLifecycle, 'strict')
const decodeFrame = Wire.decodeUnknownSync(Wire.CollectionFrame, 'strict')

type Binding = Pick<Wire.ClientControlGrant, 'grant_id' | 'item_id' | 'run_id' | 'device' | 'session'>
const sameBinding = (left: Binding, right: Binding): boolean =>
  left.grant_id === right.grant_id && left.item_id === right.item_id &&
  left.run_id === right.run_id && left.device === right.device && left.session === right.session

type RecordState = { readonly row: Wire.ClientControlGrantLifecycle; readonly terminal: boolean }
type Decision = 'activate' | 'ignore-stale-started' | 'ignore-final-grant' | 'terminate' |
  'remain-terminal' | 'ignore-item-mismatch' | 'refused-binding' | 'refused-invalid'

export interface ClientControlLifecycle {
  readonly acceptResource: (raw: unknown) => string
  readonly acceptFrame: (raw: unknown) => { readonly accepted: boolean; readonly decisions: readonly string[] }
  readonly allows: (command: Wire.DeviceCommand) => boolean
  readonly activeRunId: (grantId: string) => string | undefined
  readonly enrollmentRevoked: () => boolean
}

/** Lifecycle is a gate, never enrollment or verb/screen/budget authority. */
export const createClientControlLifecycle = ({ authority, subscriptionId }: {
  /** The host must verify the current enrollment, session, key and binding before returning a grant. */
  readonly authority: (grantId: string) => Wire.ClientControlGrant | undefined
  readonly subscriptionId: string
}): ClientControlLifecycle => {
  const records = new Map<string, RecordState>()
  let revoked = false

  const bindingRefusal = (row: Wire.ClientControlGrantLifecycle): Decision | undefined => {
    const grant = authority(row.grant_id)
    if (grant === undefined) return 'refused-binding'
    if (row.item_id !== grant.item_id) return 'ignore-item-mismatch'
    if (!sameBinding(row, grant)) return 'refused-binding'
    const previous = records.get(row.grant_id)
    if (previous !== undefined) {
      if (row.item_id !== previous.row.item_id) return 'ignore-item-mismatch'
      if (!sameBinding(row, previous.row)) return 'refused-binding'
    }
    return undefined
  }

  const apply = (row: Wire.ClientControlGrantLifecycle): Decision => {
    const previous = records.get(row.grant_id)
    if (row.state === 'revoked') revoked = true
    if (previous?.terminal === true) return row.state === 'started' ? 'ignore-final-grant' : 'remain-terminal'
    if (row.state === 'started') {
      if (previous !== undefined && row.sequence <= previous.row.sequence) return 'ignore-stale-started'
      records.set(row.grant_id, { row, terminal: false })
      return 'activate'
    }
    records.set(row.grant_id, { row, terminal: true })
    return 'terminate'
  }

  return {
    acceptResource: raw => {
      let row: Wire.ClientControlGrantLifecycle
      try { row = decodeLifecycle(raw) } catch { return 'refused-invalid' }
      return bindingRefusal(row) ?? apply(row)
    },
    acceptFrame: raw => {
      let frame: Wire.CollectionFrame
      try { frame = decodeFrame(raw) } catch { return { accepted: false, decisions: [] } }
      if ((frame.kind !== 'snapshot' && frame.kind !== 'changes') ||
        frame.collection !== 'commands' || frame.id !== subscriptionId) {
        return { accepted: false, decisions: [] }
      }
      const resources = frame.kind === 'snapshot' ? frame.items : frame.upserts
      if (resources.some(row => row.kind !== 'client-control-grant-lifecycle' && row.kind !== 'device-command')) {
        return { accepted: false, decisions: [] }
      }
      const rows = resources.filter((row): row is Wire.ClientControlGrantLifecycle => row.kind === 'client-control-grant-lifecycle')
      // Validate the whole batch before applying even a terminal event. Include bindings
      // introduced within this frame, not only records from previous frames.
      const bindings = new Map<string, Wire.ClientControlGrantLifecycle>()
      for (const row of rows) {
        if (bindingRefusal(row) !== undefined) return { accepted: false, decisions: [] }
        const prior = bindings.get(row.grant_id)
        if (prior !== undefined && !sameBinding(row, prior)) return { accepted: false, decisions: [] }
        bindings.set(row.grant_id, row)
      }
      return { accepted: true, decisions: rows.map(apply) }
    },
    allows: command => {
      if (revoked || command.grant_id === undefined || command.control_grant === undefined) return false
      const record = records.get(command.grant_id)
      if (record === undefined || record.terminal) return false
      const grant = authority(command.grant_id)
      return grant !== undefined && grant.state === 'active' && command.control_grant.state === 'active' &&
        sameBinding(record.row, grant) && sameBinding(record.row, command.control_grant) &&
        command.run_id === record.row.run_id && command.device_id === record.row.device
    },
    activeRunId: grantId => {
      const record = records.get(grantId)
      return revoked || record === undefined || record.terminal ? undefined : record.row.run_id
    },
    enrollmentRevoked: () => revoked,
  }
}
