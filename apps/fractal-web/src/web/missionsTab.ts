import * as Atom from 'effect/reactivity/Atom'

import type { DataSource } from '../data/source.ts'
import { foldedInSidebar, missionFaulted, missionNeedsYou, presentMission } from '../missions/presentation.ts'

export interface MissionTabMarkers {
  readonly count: number
  readonly needsYouCount: number
  readonly status?: { readonly glyph: '!'; readonly label: string }
}

const startOfToday = () => { const day = new Date(); day.setHours(0, 0, 0, 0); return day.getTime() }
const same = (a: MissionTabMarkers, b: MissionTabMarkers) => a.count === b.count && a.needsYouCount === b.needsYouCount && a.status?.label === b.status?.label

/** The Missions tab's pill, as its own atom so only a changed marker reaches the workspace, not every mission frame. */
export const missionTabMarkersAtom = (source: DataSource) => {
  let previous: MissionTabMarkers = { count: 0, needsYouCount: 0 }
  return Atom.make((get): MissionTabMarkers => {
    const missions = get(source.missions), agents = get(source.agents), attention = get(source.attention)
    if (missions._tag !== 'Observed') return previous
    const agentRows = agents._tag === 'Observed' ? agents.value : []
    const open = attention._tag === 'Observed' ? attention.value.filter(item => item.state === 'open') : []
    const today = startOfToday()
    let count = 0, needsYouCount = 0, faulted = 0
    for (const mission of missions.value) {
      const presentation = presentMission({ mission, agents: agentRows, attention: open })
      if (missionNeedsYou({ mission, presentation })) needsYouCount++
      if (foldedInSidebar({ mission, presentation, startOfToday: today })) continue
      count++
      if (missionFaulted(presentation)) faulted++
    }
    const next = { count, needsYouCount, ...(faulted === 0 ? {} : { status: { glyph: '!' as const, label: `${faulted} stalled or failed` } }) }
    if (!same(previous, next)) previous = next
    return previous
  })
}
