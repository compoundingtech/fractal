import type { AutoResolvedAsk } from '@fractal/client'
import * as React from 'react'

import { useAttention, useDataSource, useGrants } from '../../data/react.tsx'
import type { AlertActionResult, AutoResolvedRead, OverrideAction } from './alert-action-port.ts'

export type Override = (ask: AutoResolvedAsk, answer: OverrideAction['answer']) => Promise<AlertActionResult>

/**
 * The asks a default answered at their deadline, read when the surface opens, whenever the attention collection changes
 * (an ask that auto-resolves leaves it) and after an override is accepted; never on a timer. `read` is absent until the
 * first read answers or when the source serves none; `onOverride` is absent without an action grant.
 */
export const useAutoResolved = (): { readonly read?: AutoResolvedRead; readonly onOverride?: Override } => {
  const port = useDataSource().alertActions
  const grants = useGrants()
  const attention = useAttention()
  const [read, setRead] = React.useState<AutoResolvedRead>()
  const [accepted, setAccepted] = React.useState(0)
  // The collection itself, not its freshness: a feed that only turns stale or live is the same collection.
  const collection = attention._tag === 'Observed' ? attention.value : attention._tag
  React.useEffect(() => {
    if (port === undefined) return
    let current = true
    void port.autoResolved().then((next) => { if (current) setRead(next) })
    return () => { current = false }
  }, [port, collection, accepted])
  const onOverride: Override | undefined = port === undefined || grants.actions !== 'granted' ? undefined : (ask, answer) =>
    port.override({ ask, action: { type: 'work.override', answer } }).then((result) => {
      if (result._tag === 'Sent') setAccepted((count) => count + 1)
      return result
    })
  return { ...(read === undefined ? {} : { read }), ...(onOverride === undefined ? {} : { onOverride }) }
}
