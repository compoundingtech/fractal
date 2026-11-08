import type { Attention } from '@fractal/client/schema'
import { EntityHoverCard } from '../../../../packages/fractal-ui/src/assistant-ui/entity/EntityHoverCard.tsx'
import { SidebarStatus } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarStatus.tsx'
import { rowStatusLabel } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'
import { sidebarRow } from '../web/sidebarRow.ts'
import type { Agent, Feed } from '../data/source.ts'
import { alertTabMarkers } from '../web/alertsTab.ts'
import type { EditorTabItem } from '../workbench-kit/EditorTabs.tsx'
import { workspaceKinds } from './kinds.ts'
import type { Tab } from './model.ts'
import { parsePane } from './subject.ts'


const lastSegment = (ref: string) => ref.split('/').at(-1) ?? ref

/** A count only above one, as the TUI's tab suffix: `?`, `?2`, `◆`, `◆3`. */
const suffix = ({ questions, others }: { readonly questions: number; readonly others: number }) =>
  questions > 0 ? `?${questions > 1 ? questions : ''}` : others > 0 ? `◆${others > 1 ? others : ''}` : undefined

/** Presentation is derived on every observation, never stored: only the pane key is the tab's identity. */
export const tabPresentation = ({ tab, agents, rosterKnown, needsYou, attention, subjects }: {
  readonly tab: Tab
  readonly agents: readonly Agent[]
  readonly rosterKnown: boolean
  readonly needsYou: ReadonlySet<string>
  readonly attention: Feed<readonly Attention[]>
  readonly subjects: ReadonlyMap<string, { readonly title?: string }>
}): EditorTabItem => {
  const subject = parsePane(tab.ref)
  const base = { id: tab.id, kind: workspaceKinds.kindOf(subject).id }
  switch (subject._tag) {
    case 'Conversation':
    case 'Terminal': {
      const agent = agents.find(row => row.ref === subject.agent)
      const needsMe = needsYou.has(subject.agent)
      // The sidebar row's own status circle, amber when the agent needs you and red when it failed, and its one card.
      const row = agent === undefined ? undefined : sidebarRow({ agent, stale: false, needsMe })
      return {
        ...base,
        title: tab.title ?? agent?.name ?? lastSegment(subject.agent),
        ...(row === undefined ? {} : {
          icon: <SidebarStatus status={row.status} statusLabel={rowStatusLabel(row)} variant="Marks" needsYou={needsMe} iconOnly decorative freshness="live" />,
          hoverCard: <EntityHoverCard entity={{ kind: 'agent', id: subject.agent }} />,
        }),
        ...(needsMe ? { attention: true } : {}),
        ...(rosterKnown && agent === undefined ? { status: 'unavailable' as const } : {}),
      }
    }
    case 'Inbox': {
      const markers = alertTabMarkers(attention)
      const mark = suffix({ questions: markers.questionCount, others: markers.needsYouCount })
      return { ...base, title: tab.title ?? 'Inbox', ...(mark === undefined ? {} : { detail: mark, attention: true }) }
    }
    case 'Alert':
      return { ...base, title: tab.title ?? 'Alert' }
    case 'Missions':
      return { ...base, title: tab.title ?? 'Missions' }
    case 'Mission':
      return { ...base, title: tab.title ?? subjects.get(subject.ref)?.title ?? lastSegment(subject.ref) }
    case 'Usage':
      return { ...base, title: tab.title ?? 'Usage' }
    case 'Document':
      return { ...base, title: tab.title ?? subjects.get(subject.ref)?.title ?? lastSegment(subject.ref) }
    case 'Declaration':
      return { ...base, title: tab.title ?? `${agents.find(row => row.ref === subject.ref)?.name ?? lastSegment(subject.ref)} KDL` }
    case 'Resource':
      return { ...base, title: tab.title ?? subjects.get(subject.ref)?.title ?? lastSegment(subject.ref) }
    case 'Unsupported':
      return { ...base, title: tab.title ?? lastSegment(subject.key), status: 'unavailable' }
  }
}
