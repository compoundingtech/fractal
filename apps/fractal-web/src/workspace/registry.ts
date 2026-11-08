import { Data } from 'effect'
import type { Subject, SubjectKind } from './subject.ts'

/**
 * The typed ref to tab-kind registry: what a kind reads and how it is presented, in one place. `View` is
 * whatever the host renders with, so this module and the kit stay free of each other.
 *
 * Core kinds are closed and exhaustive. Anything else arrives as a `Resource` subject, which a contributed
 * kind (a plugin, such as GitHub) may claim by its ref and otherwise falls to the generic resource kind.
 * A contribution therefore cannot take over an agent, a mission or any other core subject.
 */
export type DataResource =
  | { readonly _tag: 'Conversation'; readonly ref: string }
  | { readonly _tag: 'TerminalScreen'; readonly ref: string }
  | { readonly _tag: 'Document'; readonly ref: string }
  | { readonly _tag: 'Declaration'; readonly ref: string }
  /** A graph resource the generic tab observes. */
  | { readonly _tag: 'Observed'; readonly ref: string }
  /** Data a contributed kind declares; the host's driver hands it to that kind. */
  | { readonly _tag: 'Contributed'; readonly kind: string; readonly ref: string }
  | { readonly _tag: 'Shared'; readonly feed: 'agents' | 'attention' | 'missions' | 'usage' }

export interface KindPolicy {
  /** Data this kind reads while visible. */
  readonly resources: (subject: Subject) => readonly DataResource[]
  /** Whether resources stay held while hidden. */
  readonly keepWarm: boolean
  /** Budget units a retained hidden DOM subtree costs; 0 means it is cheap enough never to retain. */
  readonly domCost: number
  /** Budget units its held follow costs while hidden. */
  readonly dataCost: number
}

export interface KindContribution<View> {
  /** Namespaced by its contributor, such as `github.pull-request`. */
  readonly id: string
  readonly claims: (ref: string) => boolean
  /** Refs this kind owns. The registry checks them against every contribution when it is built. */
  readonly examples: readonly [string, ...string[]]
  readonly policy: KindPolicy
  /**
   * Everything the host renders this kind with. The host's `View` type widens with what it needs (tab
   * surface, row and hover presenters over one decoded model), so there is no second registry to keep in step.
   */
  readonly view: View
}

export interface ResolvedKind<View> {
  readonly id: string
  readonly policy: KindPolicy
  readonly view: View
}

export interface KindRegistry<View> {
  readonly kindOf: (subject: Subject) => ResolvedKind<View>
}

const fleetFeeds: readonly DataResource[] = [{ _tag: 'Shared', feed: 'missions' }, { _tag: 'Shared', feed: 'agents' }, { _tag: 'Shared', feed: 'attention' }]
const attentionFeed: readonly DataResource[] = [{ _tag: 'Shared', feed: 'attention' }]

/** An agent's tab costs the same on either face. Its terminal stream follows visibility: see `desiredHolds`. */
const agentPolicy: KindPolicy = {
  resources: subject => subject._tag === 'Conversation' || subject._tag === 'Terminal' ? [{ _tag: 'Conversation', ref: subject.agent }] : [],
  keepWarm: true,
  domCost: 1,
  dataCost: 1,
}

export const corePolicies: Readonly<Record<SubjectKind, KindPolicy>> = {
  Conversation: agentPolicy,
  Terminal: agentPolicy,
  Inbox: { resources: () => attentionFeed, keepWarm: true, domCost: 1, dataCost: 0 },
  Alert: { resources: () => attentionFeed, keepWarm: true, domCost: 0, dataCost: 0 },
  Missions: { resources: () => fleetFeeds, keepWarm: true, domCost: 1, dataCost: 0 },
  Mission: { resources: () => fleetFeeds, keepWarm: true, domCost: 1, dataCost: 0 },
  Usage: { resources: () => [{ _tag: 'Shared', feed: 'usage' }], keepWarm: false, domCost: 1, dataCost: 0 },
  Document: { resources: subject => subject._tag === 'Document' ? [{ _tag: 'Document', ref: subject.ref }] : [], keepWarm: false, domCost: 1, dataCost: 1 },
  Declaration: { resources: subject => subject._tag === 'Declaration' ? [{ _tag: 'Declaration', ref: subject.ref }] : [], keepWarm: false, domCost: 1, dataCost: 1 },
  Resource: { resources: subject => subject._tag === 'Resource' ? [{ _tag: 'Observed', ref: subject.ref }] : [], keepWarm: false, domCost: 1, dataCost: 1 },
  Unsupported: { resources: () => [], keepWarm: false, domCost: 0, dataCost: 0 },
}

/** A contribution's declared example is not its alone. */
export class TabKindClaimConflict extends Data.TaggedError('TabKindClaimConflict')<{
  readonly kind: string
  readonly ref: string
  readonly claimants: readonly string[]
}> {
  override get message() {
    return `Tab kind ${this.kind} declares ${this.ref}, which is claimed by [${this.claimants.join(', ')}]`
  }
}

/**
 * Claims never resolve by order. The contribution list is static, so an overlap on a declared example
 * fails when the registry is built; a ref two kinds still both claim gets the generic resource kind.
 */
export const makeKindRegistry = <View>({ views, contributions = [] }: {
  readonly views: Readonly<Record<SubjectKind, View>>
  readonly contributions?: readonly KindContribution<View>[]
}): KindRegistry<View> => {
  for (const contribution of contributions) {
    for (const ref of contribution.examples) {
      const claimants = contributions.filter(other => other.claims(ref)).map(other => other.id)
      if (claimants.length !== 1 || claimants[0] !== contribution.id) {
        throw new TabKindClaimConflict({ kind: contribution.id, ref, claimants })
      }
    }
  }
  return {
    kindOf: subject => {
      if (subject._tag === 'Resource') {
        const claimed = contributions.filter(contribution => contribution.claims(subject.ref))
        if (claimed.length === 1) return claimed[0]!
      }
      return { id: subject._tag, policy: corePolicies[subject._tag], view: views[subject._tag] }
    },
  }
}

/** The registry as tier planning sees it, before any host supplies views. */
export const corePolicyOf = (subject: Subject): KindPolicy => corePolicies[subject._tag]
