import { appliedAnswer, notApplied, type ActionOf, type FractalClient, type OrderedMembershipOperation } from '@fractal/client'

export type QuickAccessEditIdentity = Pick<ActionOf<'arrangement.membership.edit'>, 'id' | 'idempotency_key' | 'fence'>
type Gateway = Pick<FractalClient, 'arrangementEdit' | 'arrangementMembershipEdit' | 'arrangementsGet'>
type Create = Parameters<Gateway['arrangementEdit']>[0]
type Membership = Parameters<Gateway['arrangementMembershipEdit']>[0]
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T
const details = (error: unknown): Record<string, unknown> | undefined => {
  if (typeof error !== 'object' || error === null || !('response' in error)) return undefined
  const response = error.response
  if (typeof response !== 'object' || response === null || !('details' in response)) return undefined
  return typeof response.details === 'object' && response.details !== null ? response.details as Record<string, unknown> : undefined
}
const code = (error: unknown): unknown => typeof error === 'object' && error !== null && 'response' in error
  && typeof error.response === 'object' && error.response !== null && 'code' in error.response ? error.response.code : undefined

/** Only an explicit guarantee releases the original request. Unknown takes precedence over
 * stale/rate-limited shorthand, and a replay-unsafe outcome never permits a fresh identity. */
const unsafe = (error: unknown): boolean => appliedAnswer(error) === 'unknown' || details(error)?.applied === 'unknown'
  || (details(error)?.replay_unsafe !== undefined && details(error)?.replay_unsafe !== false)
export const quickAccessNotApplied = (error: unknown): boolean => !unsafe(error) && notApplied(error)

/** One in-memory compound edit, bound to the original gateway and immutable request bodies.
 * Reads never settle ambiguous writes. A definite create race may adopt the exact live identity. */
export class QuickAccessWrite {
  stage: 'create' | 'membership'
  unconfirmed = false
  refused = false
  private adopting = false
  get needsAdoption(): boolean { return this.adopting }
  private readonly create: Create | undefined
  private readonly membership: Membership
  constructor(private readonly gateway: Gateway, selection: { owner: string; subject: string },
    identity: QuickAccessEditIdentity, operations: OrderedMembershipOperation[], createIdentity?: QuickAccessEditIdentity) {
    this.membership = clone({ ...identity, parameters: { ...selection, operations } })
    this.create = createIdentity === undefined ? undefined : clone({ ...createIdentity,
      parameters: { ...selection, version: 2, operations: [{ op: 'create', name: 'Pins' }] } })
    this.stage = this.create === undefined ? 'membership' : 'create'
  }
  async send(): Promise<void> {
    let checking = this.unconfirmed && !this.adopting
    this.refused = false
    try {
      if (this.stage === 'create') {
        if (!this.adopting) {
          try {
            const created = await this.gateway.arrangementEdit(this.create!)
            if (created.value.status !== 'completed' || created.value.action_id !== this.create!.id) throw new TypeError('Quick access creation is not confirmed')
          } catch (error) {
            // This canonical first-attempt refusal precedes the write. It has no applied field.
            if (checking || code(error) !== 'arrangement-exists' || unsafe(error)) {
              this.refused = !checking && quickAccessNotApplied(error)
              throw error
            }
            this.adopting = true
          }
        }
        if (this.adopting) {
          // A failed read retains validation, not an ambiguous CREATE. Explicit checks only GET.
          const { owner, subject } = this.membership.parameters
          const existing = (await this.gateway.arrangementsGet(owner.slice('person/'.length), subject.slice(subject.lastIndexOf('/') + 1))).value
          if (existing.id !== subject || existing.owner !== owner || existing.deleted !== false || existing.body.version !== 2)
            throw new TypeError('The reserved Quick access collection cannot be adopted')
          this.adopting = false
        }
        this.stage = 'membership'
        this.unconfirmed = false
        checking = false
      }
      try {
        const edited = await this.gateway.arrangementMembershipEdit(this.membership)
        if (edited.value.status !== 'completed' || edited.value.action_id !== this.membership.id) throw new TypeError('Quick access edit is not confirmed')
      } catch (error) { this.refused = !checking && quickAccessNotApplied(error); throw error }
      this.unconfirmed = false
    } catch (error) { this.unconfirmed = !this.refused; throw error }
  }
}
