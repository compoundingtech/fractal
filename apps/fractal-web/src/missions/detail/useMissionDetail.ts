import { useAtomValue } from '@effect/atom-react'
import type * as Fractal from '@fractal/client/schema'
import type * as Native from '@fractal/client/schema'
import * as Atom from 'effect/reactivity/Atom'
import * as React from 'react'

import { useDataSource, useNow } from '../../data/react.tsx'
import { unavailable, type Feed } from '../../data/source.ts'
import { missionDetailModel, missionRunStart, type MissionDetailModel } from './detailModel.ts'

// One mission's detail, read once for every view of it: the mission detail read (so a finished run keeps its steps),
// the missions window row while that read loads, live agents, open attention and the run's spend.

export type MissionDetailRead =
  | { readonly _tag: 'Loading' }
  /** Neither the detail read nor the window has it. */
  | { readonly _tag: 'Missing' }
  /** The window cannot be read and neither can the detail. */
  | { readonly _tag: 'WindowUnavailable'; readonly window: Feed<readonly Fractal.Mission[]> }
  | {
    readonly _tag: 'Ready'
    readonly mission: Fractal.Mission
    readonly model: MissionDetailModel
    /** The last observation is out of date. */
    readonly stale: boolean
    /** Whether the run's spend is known, still loading, or cannot be read here. */
    readonly spend: 'known' | 'loading' | 'unavailable'
  }

const notReported = Atom.make<Feed<Native.UsagePeriod>>(unavailable({ reason: 'unsupported', detail: 'This connection does not report usage.' }))

export const useMissionDetail = (missionRef: string): MissionDetailRead => {
  const source = useDataSource()
  const window = useAtomValue(source.missions)
  const detail = useAtomValue(source.subjectReads.mission.feed(missionRef))
  const agentsFeed = useAtomValue(source.agents)
  const attentionFeed = useAtomValue(source.attention)
  const now = useNow()
  const row = window._tag === 'Observed' ? window.value.find(item => item.id === missionRef) : undefined
  const mission = detail._tag === 'Observed' ? detail.value : row
  const start = mission === undefined ? undefined : missionRunStart(mission)
  const usage = useAtomValue(start === undefined || source.nativeDetails === undefined ? notReported : source.nativeDetails.missionUsage(start))
  const agents = agentsFeed._tag === 'Observed' ? agentsFeed.value : undefined
  const attention = attentionFeed._tag === 'Observed' ? attentionFeed.value : undefined
  const rows = usage._tag === 'Observed' ? usage.value.rows : undefined
  const model = React.useMemo(() => mission === undefined ? undefined
    : missionDetailModel({ mission, agents: agents ?? [], attention: (attention ?? []).filter(item => item.state === 'open'), usage: rows, now }), [mission, agents, attention, rows, now])
  if (mission === undefined || model === undefined) {
    if (detail._tag === 'Waiting' || window._tag === 'Waiting') return { _tag: 'Loading' }
    if (window._tag === 'Unavailable') return { _tag: 'WindowUnavailable', window }
    return { _tag: 'Missing' }
  }
  const stale = detail._tag === 'Observed' ? detail.freshness === 'stale' : window._tag === 'Observed' && window.freshness === 'stale'
  return { _tag: 'Ready', mission, model, stale, spend: usage._tag === 'Observed' ? 'known' : usage._tag === 'Waiting' ? 'loading' : 'unavailable' }
}
