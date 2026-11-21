import { outcomeUnknown, notApplied, type ActionOf, type FractalClient, type OrderedMembershipOperation } from '@fractal/client'

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
const unsafe = (error: unknown): boolean => outcomeUnknown(error)
  || (details(error)?.replay_unsafe !== undefined && details(error)?.replay_unsafe !== false)
export const quickAccessNotApplied = (error: unknown): boolean => !unsafe(error) && notApplied(error)

/** One in-memory compound edit, bound to the original gateway and immutable request bodies.
 * Reads never settle ambiguous writes. A definite create race may adopt the exact live identity. */
export class QuickAccessWrite {
  stage: 'create' | 'membership'
  unconfirmed = false
  refused = false
  private adopting = false
  private issue: string | undefined
  get adoptionIssue(): string | undefined { return this.issue }
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
            if (created.value.kind !== 'action-result' || created.value.status !== 'completed' || created.value.action_id !== this.create!.id) throw new TypeError('Quick access creation is not confirmed')
          } catch (error) {
            // This safe canonical conflict permits ensuring the exact shared container, not inferring our create receipt.
            if (code(error) !== 'arrangement-exists' || unsafe(error)) {
              this.refused = !checking && quickAccessNotApplied(error)
              throw error
            }
            this.adopting = true
          }
        }
        if (this.adopting) {
          // A failed read retains validation, not an ambiguous CREATE. Explicit checks only GET.
          this.issue = undefined
          const { owner, subject } = this.membership.parameters
          const existing = (await this.gateway.arrangementsGet(owner.slice('person/'.length), subject.slice(subject.lastIndexOf('/') + 1))).value
          const reason = existing.id !== subject ? 'the returned collection has another identity'
            : existing.owner !== owner ? 'the returned collection belongs to another person'
            : existing.deleted !== false ? 'the collection is retired or its live status is unavailable'
            : existing.body.version !== 2 ? 'the collection does not support ordered Quick access' : undefined
          if (reason !== undefined) {
            this.issue = `Quick access cannot be adopted because ${reason}. This collection will not be recreated.`
            throw new TypeError(this.issue)
          }
          this.adopting = false
        }
        this.stage = 'membership'
        this.unconfirmed = false
        checking = false
      }
      try {
        const edited = await this.gateway.arrangementMembershipEdit(this.membership)
        if (edited.value.kind !== 'action-result' || edited.value.status !== 'completed' || edited.value.action_id !== this.membership.id) throw new TypeError('Quick access edit is not confirmed')
      } catch (error) { this.refused = !checking && quickAccessNotApplied(error); throw error }
      this.unconfirmed = false
    } catch (error) { this.unconfirmed = !this.refused; throw error }
  }
}
