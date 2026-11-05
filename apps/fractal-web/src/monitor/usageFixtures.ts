/**
 * The Usage pane's data source over the scenario `objectives` slice: the served `slo`, `slo/history`
 * and `usage` reads, decoded like the gateway source decodes them, at the scenario clock. Tone and
 * forecast are what st served, never derived here. `read` is the client's read outcome, not served
 * data, so stories take it as an argument rather than a scenario variant.
 */
import * as Native from '@fractal/client/schema'
import type { ObjectivesState } from '@fractal/scenarios'
import * as Atom from 'effect/reactivity/Atom'
import type * as AtomRegistry from 'effect/reactivity/AtomRegistry'

import { fixtureSource } from '../data/fixtureSource.ts'
import { observed, unavailable, waiting, type DataSource, type Feed } from '../data/source.ts'
import { fixtureProjections } from '../fixtures/projections.ts'

/** No reply yet; replied; a re-read failed over the last rows; the first read failed; an older daemon. */
export const usageReads = ['live', 'waiting', 'stale', 'failed', 'unsupported'] as const
export type UsageRead = (typeof usageReads)[number]

const failure = { reason: 'failed', detail: 'Could not read objectives and budgets.' } as const

export const scenarioUsageSource = ({ state, loading, read, now, registry }: {
  readonly state: ObjectivesState
  /** The slice withholds its first reply. */
  readonly loading: boolean
  readonly read: UsageRead
  readonly now: number
  readonly registry: AtomRegistry.AtomRegistry
}): DataSource => {
  const list = Native.decodeUnknownSync(Native.SloList)(state.list)
  const usage = Native.decodeUnknownSync(Native.UsagePeriod)(state.usage)
  const histories = new Map(state.histories.map(history => [history.id, Native.decodeUnknownSync(Native.SloHistory)(history)]))
  const feed = <A>(value: A): Feed<A> =>
    loading || read === 'waiting' ? waiting
      : read === 'failed' ? unavailable(failure)
        : read === 'unsupported' ? unavailable({ reason: 'unsupported', detail: 'This daemon does not serve objectives and budgets.' })
          : read === 'stale' ? { _tag: 'Observed', value, freshness: 'stale', error: failure } : observed({ value })
  const listAtom = Atom.make(feed(list))
  const accountsAtom = Atom.make(feed(usage))
  return {
    ...fixtureSource({ world: fixtureProjections }),
    now: Atom.make(now),
    objectives: {
      list: listAtom,
      accounts: accountsAtom,
      history: Atom.family((id: string) => Atom.make<Feed<Native.SloHistory>>(histories.has(id) ? observed({ value: histories.get(id)! }) : unavailable(failure))),
      refresh: () => {
        registry.set(listAtom, observed({ value: list }))
        registry.set(accountsAtom, observed({ value: usage }))
      },
    },
  }
}
