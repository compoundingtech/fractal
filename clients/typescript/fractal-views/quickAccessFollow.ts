import type { Arrangement, CollectionStream, FractalClient, OrderedMembership, OrderedMembershipOperation } from '@fractal/client'
import { quickAccessSelection, quickAccessDestination, validQuickAccessKey } from './quickAccess.ts'
import { QuickAccessWrite, type QuickAccessEditIdentity as EditIdentity } from './quickAccessWrite.ts'

export type QuickAccessGateway = Pick<FractalClient, 'discover' | 'arrangementsGet' | 'arrangementsMemberships' | 'arrangementEdit' | 'arrangementMembershipEdit' | 'collectionStream'>
export interface QuickAccessFollow {
  close(): void
  refresh(): Promise<void>
  checkEdit(): Promise<void>
  edit(identity: EditIdentity, operations: OrderedMembershipOperation[], createIdentity?: EditIdentity): Promise<void>
}
export interface QuickAccessState {
  readonly phase: 'connecting' | 'synced' | 'pending' | 'unavailable' | 'unconfirmed'
  readonly members: readonly OrderedMembership[]
  readonly detail?: string
  readonly editable?: boolean
  readonly unconfirmed?: { readonly stage: 'create' | 'membership' }
}
const record = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value)
const isArrangement = (value: unknown): value is Arrangement => record(value) && typeof value.id === 'string' && typeof value.owner === 'string'
  && typeof value.revision === 'string' && value.deleted === false && record(value.body) && value.body.version === 2
  && record(value.body.name) && typeof value.body.name.value === 'string' && record(value.body.folders)
const isMember = (value: unknown): value is OrderedMembership => record(value) && typeof value.member === 'string' && value.id === value.member
  && typeof value.container === 'string' && typeof value.revision === 'string' && record(value.position)
  && value.position.bucket === null && typeof value.position.key === 'string' && validQuickAccessKey(value.position.key)
const isPage = (value: unknown): value is { items: OrderedMembership[]; page: { has_more: boolean; next_cursor?: string | null } } => record(value)
  && value.kind === 'page' && value.collection === 'ordered-memberships' && Array.isArray(value.items) && value.items.every(isMember)
  && record(value.page) && typeof value.page.has_more === 'boolean'
  && (value.page.next_cursor === undefined || value.page.next_cursor === null || typeof value.page.next_cursor === 'string')
const validOperations = (operations: readonly OrderedMembershipOperation[]): boolean => operations.length > 0 && operations.length <= 1024
  && new Set(operations.map(operation => operation.member)).size === operations.length
  && new TextEncoder().encode(JSON.stringify(operations)).length <= 1048576
  && operations.every(operation => typeof operation.member === 'string' && operation.member.length > 0 && (
    operation.op === 'remove' || operation.op === 'place' && quickAccessDestination(operation.member) !== null
      && operation.bucket === null && validQuickAccessKey(operation.key)))
const hasCode = (error: unknown, code: string): boolean => typeof error === 'object' && error !== null &&
  'response' in error && typeof error.response === 'object' && error.response !== null && 'code' in error.response && error.response.code === code
type FollowOptions = {
  readonly gateway: QuickAccessGateway
  readonly onState: (state: QuickAccessState) => void
  readonly signal?: AbortSignal
  /** The owner must coordinate all arrangement readers before enabling v2 creation. */
  readonly v2ReadersReady?: boolean
}
interface Session { refuse(error: unknown): void; close(): void; refresh(): Promise<void>; needsReconnect(): boolean; wasObserved(): boolean; prepare(identity: EditIdentity, operations: OrderedMembershipOperation[], createIdentity?: EditIdentity): QuickAccessWrite }

const openSession = async ({ gateway, onState, signal, v2ReadersReady = false }: FollowOptions, previous: { observed: boolean; members: readonly OrderedMembership[] }): Promise<Session> => {
  const discovery = (await gateway.discover(signal === undefined ? {} : { signal })).value
  const selection = quickAccessSelection(discovery.session_actor)
  const person = selection.owner.slice('person/'.length), uuid = selection.subject.slice(selection.subject.lastIndexOf('/') + 1)
  if (!discovery.capabilities.some((item) => item.id === 'arrangements' && item.version === 2 && item.state === 'granted'))
    throw new TypeError('Pins require granted arrangements version 2')
  const writable = discovery.capabilities.some((item) => item.id === 'control.arrangements' && item.state === 'granted')
  let closed = false
  let interrupted = false
  let reading = false
  let dirty = false
  let ready = false
  let observed = previous.observed
  let retired = false
  let arrangement: Arrangement | undefined
  let state: QuickAccessState = { phase: 'connecting', members: previous.members }
  let stream: CollectionStream | undefined
  const abort = new AbortController()
  const close = () => { closed = true; abort.abort(); stream?.close(); signal?.removeEventListener('abort', close) }
  if (signal?.aborted) throw new TypeError('Quick access follow aborted')
  signal?.addEventListener('abort', close, { once: true })
  const publish = (next: QuickAccessState) => {
    state = { ...next, editable: next.phase === 'synced' && ready && writable && !interrupted && !retired && (arrangement !== undefined || v2ReadersReady) }
    if (!closed) onState(state)
  }
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
        const memberIds = new Set<string>()
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
            for (const item of page.items) {
              if (memberIds.has(item.member)) throw new TypeError('Duplicate Pins membership across pages')
              memberIds.add(item.member)
            }
            members.push(...page.items)
            cursor = page.page.has_more ? page.page.next_cursor ?? undefined : undefined
            if (page.page.has_more && (cursor === undefined || seen.has(cursor))) throw new TypeError('Invalid Pins continuation cursor')
            if (cursor !== undefined) seen.add(cursor)
          } while (cursor !== undefined)
        }
        if (!closed && !dirty && !interrupted) {
          ready = !retired
          publish({ phase: retired ? 'unavailable' : 'synced', members, editable: writable && (next !== undefined || v2ReadersReady), ...(retired ? { detail: 'Restoring removed Pins is not available.' } : {}) })
        }
      } while (dirty && !closed)
    } catch (error) { if (!closed) fail(error) } finally { reading = false }
  }
  const read = (): Promise<void> => {
    if (reading) { dirty = true; return pendingRead ?? Promise.resolve() }
    pendingRead = runRead()
    return pendingRead
  }
  const refresh = () => read()
  publish(state)
  try {
    stream = await gateway.collectionStream({
      onFrame: (frame) => {
        if (frame.kind === 'error') { interrupt(new Error(frame.message)); return }
        if ((frame.kind === 'snapshot' || frame.kind === 'changes') && (frame.id === 'pins-layout' || frame.id === 'pins-members')) void read()
      },
      onEnd: (error) => interrupt(error ?? new Error('Quick access updates disconnected. Refresh Quick access to reconnect.')),
    })
    if (closed) stream.close()
    else {
      stream.subscribeArrangements('pins-layout', selection.owner, 1, selection.subject)
      await read()
    }
  } catch (error) { interrupt(error) }
  return {
    close,
    refuse: error => { if (hasCode(error, 'arrangement-retired')) { retired = true; observed = true } },
    refresh,
    needsReconnect: () => interrupted,
    wasObserved: () => observed,
    prepare: (identity, operations, createIdentity) => {
      if (closed || !ready || !writable) throw new TypeError('Pins must be synced, writable and idle before editing')
      if (!validOperations(operations)) throw new TypeError('Invalid Pins membership operations')
      if (arrangement === undefined) {
        if (!v2ReadersReady || createIdentity === undefined) throw new TypeError('Creating Pins requires coordinated v2 readers and a separate create identity')
        if (createIdentity.id === identity.id || createIdentity.idempotency_key === identity.idempotency_key) throw new TypeError('Create and membership actions require distinct identities')
      }
      return new QuickAccessWrite(gateway, selection, identity, operations, arrangement === undefined ? createIdentity : undefined)
    },
  }
}

/** Refresh explicitly reopens an interrupted follow. It never replays a membership edit. */
export const followQuickAccess = async (options: FollowOptions): Promise<QuickAccessFollow> => {
  let closed = false, generation = 0, observed = false, editing = false
  let current: Session | undefined, opening: Promise<void> | undefined, sessionAbort: AbortController | undefined
  let pending: QuickAccessWrite | undefined
  let latest: QuickAccessState = { phase: 'connecting', members: [] }
  const present = () => {
    if (closed) return
    options.onState(pending?.unconfirmed ? { ...latest, phase: editing ? 'pending' : 'unconfirmed', editable: false, unconfirmed: { stage: pending.stage }, detail: pending.needsAdoption ? pending.adoptionIssue ?? 'Quick access already exists. Check delivery to validate the shared collection before sending your edit.' : 'Fractal may have changed Quick access. Check delivery before making another edit.' }
      : editing ? { ...latest, phase: 'pending', editable: false } : latest)
  }
  const close = () => { closed = true; generation++; sessionAbort?.abort(); current?.close(); current = undefined; options.signal?.removeEventListener('abort', close) }
  if (options.signal?.aborted) throw new TypeError('Quick access follow aborted')
  options.signal?.addEventListener('abort', close, { once: true })
  const reopen = (): Promise<void> => {
    if (closed) return Promise.resolve()
    if (opening) return opening
    const version = ++generation
    observed ||= current?.wasObserved() ?? false
    current?.close(); current = undefined; sessionAbort?.abort(); sessionAbort = new AbortController()
    const publish = (state: QuickAccessState) => { if (!closed && version === generation) { latest = state; present() } }
    publish({ phase: 'connecting', members: latest.members, editable: false })
    const task = openSession({ ...options, signal: sessionAbort.signal, onState: publish }, { observed, members: latest.members })
      .then(next => { if (closed || version !== generation) next.close(); else current = next })
      .catch(error => publish({ phase: 'unavailable', members: latest.members, editable: false, detail: error instanceof Error ? error.message : String(error) }))
    opening = task
    void task.then(() => { if (opening === task) opening = undefined })
    return task
  }
  const sendPending = async () => {
    if (closed || editing || !pending) throw new TypeError('There is no idle Quick access edit to check')
    const write = pending
    editing = true; present()
    try {
      await write.send(); pending = undefined
      await current?.refresh()
    } catch (error) {
      if (write.refused) { pending = undefined; current?.refuse(error); await current?.refresh() }
      throw error
    } finally { editing = false; present() }
  }
  await reopen()
  return {
    close,
    checkEdit: sendPending,
    refresh: () => closed ? Promise.resolve() : opening ?? (current && (!current.needsReconnect() || editing) ? current.refresh() : reopen()),
    edit: async (identity, operations, createIdentity) => {
      if (closed || opening || editing || pending || !current) throw new TypeError('Pins must be synced, writable and idle before editing')
      pending = current.prepare(identity, operations, createIdentity)
      await sendPending()
    },
  }
}
