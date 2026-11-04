import { PINS_ARRANGEMENT_UUID } from '@fractal/client'
import type { ActionOf, Arrangement, CollectionStream, FractalClient, OrderedMembership, OrderedMembershipOperation } from '@fractal/client'
import { Schema } from 'effect'
import { Arrangement as ArrangementSchema, OrderedMembershipPage, ArrangementMembershipEditParameters } from '@fractal/client/schema'
import { selectionParts } from '../folders/client.ts'

export type PinsGateway = Pick<FractalClient, 'discover' | 'arrangementsGet' | 'arrangementsMemberships' | 'arrangementEdit' | 'arrangementMembershipEdit' | 'collectionStream'>
type EditIdentity = Pick<ActionOf<'arrangement.membership.edit'>, 'id' | 'idempotency_key' | 'fence'>
export interface PinsState {
  readonly phase: 'connecting' | 'synced' | 'pending' | 'unavailable'
  readonly members: readonly OrderedMembership[]
  readonly detail?: string
}
const isArrangement = Schema.is(Schema.toEncoded(ArrangementSchema))
const isPage = Schema.is(Schema.toEncoded(OrderedMembershipPage))
const isEdit = Schema.is(Schema.toEncoded(ArrangementMembershipEditParameters))
const hasCode = (error: unknown, code: string): boolean => typeof error === 'object' && error !== null &&
  'response' in error && typeof error.response === 'object' && error.response !== null && 'code' in error.response && error.response.code === code
export const pinsSelection = (actor: string): { owner: string; subject: string } => {
  const owner = /^(person\/[^/\s]+)(?:\/session\/[^/\s]+)?$/.exec(actor)?.[1]
  if (owner === undefined) throw new TypeError('Pins require a discovered person principal')
  return { owner, subject: `arrangement/${owner}/${PINS_ARRANGEMENT_UUID}` }
}

/** Online edits use caller-owned immutable IDs/fences. No offline queue or implicit migration. */
export const followPins = async ({ gateway, onState, v2ReadersReady = false }: {
  readonly gateway: PinsGateway
  readonly onState: (state: PinsState) => void
  /** The owner must coordinate all arrangement readers before enabling v2 creation. */
  readonly v2ReadersReady?: boolean
}): Promise<{ close(): void; refresh(): void; edit(identity: EditIdentity, operations: OrderedMembershipOperation[], createIdentity?: EditIdentity): Promise<void> }> => {
  const discovery = (await gateway.discover()).value
  const selection = pinsSelection(discovery.session_actor)
  const { person, uuid } = selectionParts(selection)
  if (!discovery.capabilities.some((item) => item.id === 'arrangements' && item.version === 2 && item.state === 'granted'))
    throw new TypeError('Pins require granted arrangements version 2')
  const writable = discovery.capabilities.some((item) => item.id === 'control.arrangements' && item.state === 'granted')
  let closed = false
  let interrupted = false
  let reading = false
  let dirty = false
  let ready = false
  let busy = false
  let observed = false
  let retired = false
  let arrangement: Arrangement | undefined
  let state: PinsState = { phase: 'connecting', members: [] }
  let stream: CollectionStream | undefined
  const abort = new AbortController()
  const publish = (next: PinsState) => { state = next; if (!closed) onState(next) }
  const fail = (error: unknown) => { ready = false; publish({ ...state, phase: 'unavailable', detail: error instanceof Error ? error.message : String(error) }) }
  const interrupt = (error: unknown) => { interrupted = true; fail(error) }
  let membershipSubscribed = false
  let pendingRead: Promise<void> | undefined
  const runRead = async (): Promise<void> => {
    if (closed) return
    reading = true
    try {
      do {
        dirty = false
        let next: Arrangement | undefined
        try {
          next = (await gateway.arrangementsGet(person, uuid, { signal: abort.signal })).value
          if (!isArrangement(next) || next.id !== selection.subject || next.owner !== selection.owner || next.body.version !== 2)
            throw new TypeError('Pins identity must contain a version-2 arrangement for the principal')
        } catch (error) { if (!hasCode(error, 'not-found')) throw error }
        if (closed) return
        retired ||= observed && next === undefined
        observed ||= next !== undefined
        arrangement = next
        const members: OrderedMembership[] = []
        if (next !== undefined) {
          if (stream !== undefined && !membershipSubscribed) {
            stream.subscribeOrderedMemberships('pins-members', selection.owner, selection.subject, Math.min(200, discovery.limits.max_page_items))
            membershipSubscribed = true
          }
          let cursor: string | undefined
          const seen = new Set<string>()
          do {
            const page = (await gateway.arrangementsMemberships(person, uuid, selection.owner, { limit: Math.min(200, discovery.limits.max_page_items), ...(cursor === undefined ? {} : { cursor }) }, { signal: abort.signal })).value
            if (!isPage(page) || page.items.some((item) => item.container !== selection.subject || item.position.bucket !== null))
              throw new TypeError('Invalid root Pins memberships')
            members.push(...page.items)
            cursor = page.page.has_more ? page.page.next_cursor ?? undefined : undefined
            if (page.page.has_more && (cursor === undefined || seen.has(cursor))) throw new TypeError('Invalid Pins continuation cursor')
            if (cursor !== undefined) seen.add(cursor)
          } while (cursor !== undefined)
        }
        if (!closed && !dirty && !interrupted) {
          ready = !retired
          publish({ phase: retired ? 'unavailable' : busy ? 'pending' : 'synced', members, ...(retired ? { detail: 'Restoring removed Pins is not available.' } : {}) })
        }
      } while (dirty && !closed)
    } catch (error) { if (!closed) fail(error) } finally { reading = false }
  }
  const read = (): Promise<void> => {
    if (reading) { dirty = true; return pendingRead ?? Promise.resolve() }
    pendingRead = runRead()
    return pendingRead
  }
  const refresh = () => { void read() }
  publish(state)
  try {
    stream = await gateway.collectionStream({
      onFrame: (frame) => {
        if (frame.kind === 'error') { interrupt(new Error(frame.message)); return }
        if ((frame.kind === 'snapshot' || frame.kind === 'changes') && (frame.id === 'pins-layout' || frame.id === 'pins-members')) refresh()
      },
      onEnd: (error) => interrupt(error ?? new Error('Pins updates disconnected; reopen the Pins follow.')),
    })
    stream.subscribeArrangements('pins-layout', selection.owner, 1, selection.subject)
    await read()
  } catch (error) { interrupt(error) }
  return {
    close: () => { closed = true; abort.abort(); stream?.close() },
    refresh,
    edit: async (identity, operations, createIdentity) => {
      if (closed || !ready || busy || !writable) throw new TypeError('Pins must be synced, writable and idle before editing')
      const parameters = { ...selection, operations }
      if (!isEdit(parameters)) throw new TypeError('Invalid Pins membership operations')
      if (operations.some((operation) => operation.op === 'place' && operation.bucket !== null)) throw new TypeError('Pins use only the root bucket')
      busy = true
      publish({ ...state, phase: 'pending' })
      try {
        if (arrangement === undefined) {
          if (!v2ReadersReady || createIdentity === undefined) throw new TypeError('Creating Pins requires coordinated v2 readers and a separate create identity')
          if (createIdentity.id === identity.id || createIdentity.idempotency_key === identity.idempotency_key) throw new TypeError('Create and membership actions require distinct identities')
          try { await gateway.arrangementEdit({ ...createIdentity, parameters: { ...selection, version: 2, operations: [{ op: 'create', name: 'Pins' }] } }) }
          catch (error) { if (hasCode(error, 'arrangement-retired')) retired = true; if (!hasCode(error, 'arrangement-exists')) throw error }
          await read()
          if (arrangement === undefined || retired || !ready) throw new TypeError('Reserved Pins is unavailable')
        }
        await gateway.arrangementMembershipEdit({ ...identity, parameters })
        await read()
      } catch (error) { fail(error); throw error } finally { busy = false; if (ready) publish({ ...state, phase: 'synced' }) }
    },
  }
}
