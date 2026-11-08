import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { EntityStatusMark } from '../../../../packages/fractal-ui/src/assistant-ui/entity/EntityCard.tsx'
import { EntityHoverCard } from '../../../../packages/fractal-ui/src/assistant-ui/entity/EntityHoverCard.tsx'
import { SidebarStatus } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarStatus.tsx'
import { rowStatusLabel } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'
import { sidebarRow } from '../web/sidebarRow.ts'
import { alertTabMarkers } from '../web/alertsTab.ts'
import type { EditorTabItem } from '../workbench-kit/EditorTabs.tsx'
import { unreadLabel } from '../conversation/unread.ts'
import { workspaceKinds } from './kinds.ts'
import type { Tab } from './model.ts'
import { titleText, type KindDescriptor, type Mark, type PresentationWorld } from './presentation.ts'
import { parsePane, type Subject } from './subject.ts'

/** A count only above one, as the TUI's tab suffix: `?`, `?2`, `◆`, `◆3`. */
const suffix = ({ questions, others }: { readonly questions: number; readonly others: number }) =>
  questions > 0 ? `?${questions > 1 ? questions : ''}` : others > 0 ? `◆${others > 1 ? others : ''}` : undefined

/** The one descriptor of a subject, through its kind. Every surface that names a subject asks here. */
export const describe = (subject: Subject, world: PresentationWorld): KindDescriptor =>
  workspaceKinds.kindOf(subject).present(subject, world)

/** The printed title: a pending one shows its kind word until the data arrives. */
export const describedTitle = (descriptor: KindDescriptor): string =>
  `${titleText(descriptor.title)}${descriptor.title._tag === 'Pending' ? '…' : ''}`

/** The mark as drawn: an agent's own sidebar ring, a status mark, or the kind's icon. */
const markNode = (mark: Mark, world: PresentationWorld) => {
  switch (mark._tag) {
    case 'Agent': {
      const agent = world.agents.find(row => row.ref === mark.agent)
      if (agent === undefined) return undefined
      const needsMe = world.needsYou.has(mark.agent)
      const row = sidebarRow({ agent, stale: false, needsMe })
      return <SidebarStatus status={row.status} statusLabel={rowStatusLabel(row)} variant="Marks" needsYou={needsMe} iconOnly decorative freshness="live" />
    }
    case 'Status': return <EntityStatusMark status={mark.status} />
    case 'Icon': return <Icon name={mark.name} size={12} />
  }
}

/** Presentation is derived on every observation, never stored: only the pane key is the tab's identity. */
export const tabPresentation = ({ tab, world, unread }: {
  readonly tab: Tab
  readonly world: PresentationWorld
  /** New prose since the person last looked, by agent ref: a count, or `some` when only a newer time is known. */
  readonly unread?: ReadonlyMap<string, number | 'some'>
}): EditorTabItem => {
  const subject = parsePane(tab.ref)
  const descriptor = describe(subject, world)
  const icon = markNode(descriptor.mark, world)
  const item: EditorTabItem = {
    id: tab.id,
    kind: workspaceKinds.kindOf(subject).id,
    // A person's own name for the tab wins.
    title: tab.title ?? describedTitle(descriptor),
    ...(icon === undefined ? {} : { icon }),
    ...(descriptor.card === null ? {} : { hoverCard: <EntityHoverCard entity={descriptor.card} /> }),
    ...(descriptor.title._tag === 'Gone' ? { status: 'unavailable' as const } : {}),
  }
  switch (subject._tag) {
    case 'Conversation':
    case 'Terminal': {
      const needsMe = world.needsYou.has(subject.agent)
      const unseen = unread?.get(subject.agent)
      return {
        ...item,
        ...(needsMe ? { attention: true } : {}),
        // Needs-you keeps the slot; unread shows only on the conversation itself.
        ...(!needsMe && subject._tag === 'Conversation' && unseen !== undefined ? { detail: unreadLabel(unseen) } : {}),
      }
    }
    case 'Inbox': {
      const markers = alertTabMarkers(world.attention)
      const mark = suffix({ questions: markers.questionCount, others: markers.needsYouCount })
      return mark === undefined ? item : { ...item, detail: mark, attention: true }
    }
    case 'Alert':
      return descriptor.mark._tag === 'Status' && descriptor.mark.status.tone === 'needs-you' ? { ...item, attention: true } : item
    default:
      return item
  }
}
