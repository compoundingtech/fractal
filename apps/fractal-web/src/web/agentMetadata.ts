import type { Attention } from '@fractal/client/schema'
import { Option } from 'effect'
import { alertConversations } from '../../../../clients/typescript/fractal-views/homeView.ts'
import type { NativeDeclarations } from '../data/nativeDeclarations.ts'
import type { Feed } from '../data/source.ts'

/**
 * A person ask (`work.ask`) shown in this agent's conversation: an open person-step card that is not an update. The
 * attention feed is already scoped to the current person; gates, approvals, prompts and runs waiting on the person are
 * not asks.
 */
export const isOpenPersonAsk = ({ item, agentRef }: { readonly item: Attention; readonly agentRef: string }): boolean =>
  item.state === 'open' && item.attention_kind === 'person-step' && item.update === undefined &&
  alertConversations(item).includes(agentRef)

/** Exact over complete attention only: waiting, unavailable or explicitly bounded evidence is none, never a lower bound. */
export const openPersonAskCount = ({ feed, agentRef }: {
  readonly feed: Feed<readonly Attention[]>; readonly agentRef: string
}): Option.Option<number> =>
  feed._tag === 'Observed' && feed.coverage === undefined
    ? Option.some(feed.value.filter(item => isOpenPersonAsk({ item, agentRef })).length)
    : Option.none()

/** The harness effort the read declaration asks for; none while unread, refused, rejected or undeclared. */
export const declaredEffort = (feed: Feed<NativeDeclarations>): Option.Option<string> =>
  feed._tag === 'Observed' ? Option.fromUndefinedOr(feed.value.declaredEffort) : Option.none()
