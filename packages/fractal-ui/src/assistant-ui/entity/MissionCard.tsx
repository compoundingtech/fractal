/**
 * Mission content in the shared card grammar: the Missions section an agent card carries, and the standalone mission
 * card a mission row, a mission tab or a mission mention shows. Both read the same sidebar mission model, so a mission
 * says the same title, progress and step wherever it is hovered.
 */
import * as React from 'react'
import { HumanClockProvider, HumanTime, fixedClock } from '../../time/HumanTime'
import type { SidebarMissionContext, SidebarMissionInfo, SidebarMissionProgress, SidebarMissionStep, SidebarOtherMission } from '../sidebar/model'
import type { SidebarClock } from '../sidebar/SidebarTime'
import { EntityCard, EntityCardHeader, EntityCardList, EntityCardListItem, EntityCardMeta, EntityCardMetaLine, EntityCardNote, EntityCardSection, EntityCardText, EntityStatusMark } from './EntityCard'
import type { EntityDescriptor, EntityStatus } from './entity.ts'

/** The card lists about this many missions; Reported details lists them all for keyboard users. */
const cardMissionLimit = 5
export const missionStepText = (step: SidebarMissionStep) => step._tag === 'Step' ? step.name : step._tag === 'Waiting' ? `Waiting for you: ${step.question}` : undefined
export const missionProgressText = (progress: SidebarMissionProgress) => progress._tag === 'Known' ? `${progress.done}/${progress.total}` : undefined
type CurrentMission = Extract<SidebarMissionContext, { _tag: 'Mission' }>

export const missionStatus = (mission: Pick<SidebarOtherMission, 'step' | 'progress'>): EntityStatus | undefined =>
  mission.step._tag === 'Waiting' ? { tone: 'needs-you', label: 'Waiting for you' }
    : mission.progress._tag === 'Known' && mission.progress.total > 0 && mission.progress.done >= mission.progress.total ? { tone: 'done', label: 'Done' }
      : mission.step._tag === 'Step' ? { tone: 'working', label: 'Working' } : undefined

export const missionDescriptor = (mission: SidebarOtherMission): EntityDescriptor => {
  const status = missionStatus(mission)
  return { kind: 'mission', id: mission.id, title: mission.title ?? mission.slug, ...(status === undefined ? {} : { status }) }
}

/** The missions below are the retained last observation when their feed is Stale or Failed, even under a live agent status. */
export function MissionInfoNote({ info, now }: { readonly info: SidebarMissionInfo } & SidebarClock) {
  if (info._tag === 'Live') return null
  const note = <EntityCardNote data-testid="hover-missions-note" data-mission-info-note={info._tag.toLowerCase()}>
    {info._tag === 'Stale' ? 'Mission info may be out of date.' : 'Couldn’t refresh mission info.'}
    {info.updatedAt !== undefined && <> Updated <HumanTime at={info.updatedAt} kind="elapsed" maxUnits={1} suffix=" ago" />.</>}
  </EntityCardNote>
  return now === undefined ? note : <HumanClockProvider clock={fixedClock(now)}>{note}</HumanClockProvider>
}

const MissionItem = ({ title, step, progress }: { readonly title: string; readonly step?: string; readonly progress?: string }) =>
  <EntityCardListItem title={title} trailing={progress} detail={step !== undefined && step !== title ? step : undefined} />

/** The missions an agent holds now, then those queued behind them, in reported order. */
export function MissionsSection({ context, info, now }: { readonly context: SidebarMissionContext | undefined; readonly info?: SidebarMissionInfo | undefined } & SidebarClock) {
  if (context?._tag !== 'Mission') return null
  const others = (context.alsoCurrent ?? []).slice(0, cardMissionLimit - 1)
  const queued = context.queued.slice(0, cardMissionLimit - 1 - others.length)
  const more = (context.alsoCurrent?.length ?? 0) + context.queued.length - others.length - queued.length
  return <EntityCardSection label="Missions" icon="checklist" testId="hover-missions">
    {info !== undefined && <MissionInfoNote info={info} now={now} />}
    <EntityCardList>
      <MissionItem title={context.title ?? context.slug} step={missionStepText(context.step)} progress={missionProgressText(context.progress)} />
      {others.map(mission => <MissionItem key={mission.id} title={mission.title ?? mission.slug} step={missionStepText(mission.step)} progress={missionProgressText(mission.progress)} />)}
    </EntityCardList>
    {queued.length > 0 && <>
      <EntityCardNote>Queued</EntityCardNote>
      <EntityCardList>{queued.map(mission => <MissionItem key={mission.id} title={mission.title ?? mission.slug} />)}</EntityCardList>
    </>}
    {more > 0 && <EntityCardNote>+{more} more</EntityCardNote>}
  </EntityCardSection>
}

/** The current mission's own title and step can already be the agent's reported goal; the agent card does not repeat it. */
export const missionsSay = (context: SidebarMissionContext | undefined, text: string) =>
  context?._tag === 'Mission' && (context.title === text || missionStepText(context.step) === text)

export interface MissionEntityCardProps extends SidebarClock {
  readonly mission: SidebarOtherMission | CurrentMission
  readonly info?: SidebarMissionInfo
  /** The agents working on it, by title. */
  readonly agents?: readonly string[]
  /** Missions queued behind it on the same agent. */
  readonly queued?: readonly string[]
}

/** A mission on its own: progress and step lead, then who works on it. */
export function MissionEntityCard({ mission, info, agents = [], queued = [], now }: MissionEntityCardProps) {
  const descriptor = missionDescriptor(mission)
  const step = missionStepText(mission.step)
  const progress = missionProgressText(mission.progress)
  return <EntityCard entity={descriptor} testId="mission-hover-card">
    <EntityCardHeader title={descriptor.title} kind="Mission"
      glyph={descriptor.status === undefined ? undefined : <EntityStatusMark status={descriptor.status} />}
      summary={[descriptor.status?.label, progress === undefined ? undefined : `${progress} steps`, mission.title !== undefined && mission.slug !== mission.title ? mission.slug : undefined]} />
    {info !== undefined && <MissionInfoNote info={info} now={now} />}
    {step !== undefined && <EntityCardText icon="checklist">{step}</EntityCardText>}
    {agents.length > 0 && <EntityCardSection label="Agents" icon="message"><EntityCardList>{agents.map(agent => <EntityCardListItem key={agent} title={agent} />)}</EntityCardList></EntityCardSection>}
    {queued.length > 0 && <EntityCardMeta><EntityCardMetaLine icon="clock" label="Queued behind">{queued.join(', ')}</EntityCardMetaLine></EntityCardMeta>}
  </EntityCard>
}
