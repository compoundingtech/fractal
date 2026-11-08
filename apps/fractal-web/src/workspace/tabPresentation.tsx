import type { Attention } from '@fractal/client/schema'
import type { Agent, Feed } from '../data/source.ts'
import { alertTabMarkers } from '../web/alertsTab.ts'
import type { EditorTabItem } from '../workbench-kit/EditorTabs.tsx'
import { workspaceKinds } from './kinds.ts'
import type { Tab } from './model.ts'
import { parsePane } from './subject.ts'

/** `docs/clients/ui-contract.md`: one activity glyph before an agent tab's title. */
const activityGlyph = (agent: Agent | undefined): string => {
  if (agent === undefined) return '~'
  if (agent.activity === 'errored') return '!'
  if (!agent.connected) return '×'
  return agent.activity === 'working' ? '●' : '○'
}

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
  const subject = parsePane(tab.pane)
  const base = { id: tab.id, kind: workspaceKinds.kindOf(subject).id }
  switch (subject._tag) {
    case 'Conversation':
    case 'Terminal': {
      const agent = agents.find(row => row.ref === subject.agent)
      const mark = suffix({ questions: agent?.ask._tag === 'Known' && agent.ask.value === 'question' ? 1 : 0, others: needsYou.has(subject.agent) ? 1 : 0 })
      return {
        ...base,
        title: tab.title ?? agent?.name ?? lastSegment(subject.agent),
        icon: <span aria-hidden="true">{activityGlyph(agent)}</span>,
        ...(mark === undefined ? {} : { detail: mark, attention: true }),
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
