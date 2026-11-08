import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { EntityStatusMark } from '../../../../packages/fractal-ui/src/assistant-ui/entity/EntityCard.tsx'
import { EntityHoverCard } from '../../../../packages/fractal-ui/src/assistant-ui/entity/EntityHoverCard.tsx'
import { SidebarStatus } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarStatus.tsx'
import { rowStatusLabel } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'
import { sidebarRow } from '../web/sidebarRow.ts'
import { alertTabMarkers } from '../web/alertsTab.ts'
import type { EditorTabItem } from '../workbench-kit/EditorTabs.tsx'
import { unreadLabel } from '../conversation/unread.ts'
import { describe, workspaceKinds } from './kinds.ts'
import type { Tab } from './model.ts'
import { titleText, type KindDescriptor, type Mark, type PresentationWorld } from './presentation.ts'
import { parsePane } from './subject.ts'

/** A count only above one, as the TUI's tab suffix: `?`, `?2`, `◆`, `◆3`. */
const suffix = ({ questions, others }: { readonly questions: number; readonly others: number }) =>
  questions > 0 ? `?${questions > 1 ? questions : ''}` : others > 0 ? `◆${others > 1 ? others : ''}` : undefined

/** The printed title. A pending or gone title prints the last one this client saw, else its kind word. */
export const describedTitle = (descriptor: KindDescriptor, seen?: string): string =>
  descriptor.title._tag === 'Known' || descriptor.title._tag === 'Fixed' ? descriptor.title.text : seen ?? titleText(descriptor.title)

/** The name worth remembering for a tab: only one observed in data, never a placeholder. */
export const rememberedTitle = (descriptor: KindDescriptor): string | undefined =>
  descriptor.title._tag === 'Known' ? descriptor.title.text : undefined

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
export const tabPresentation = ({ tab, world, seen, unread }: {
  readonly tab: Tab
  readonly world: PresentationWorld
  /** The last title this client saw for the tab (`TabView.seenTitle`): shown while loading and once gone. */
  readonly seen?: string | undefined
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
    title: tab.title ?? describedTitle(descriptor, seen),
    ...(icon === undefined ? {} : { icon }),
    // Only a named entity has a card to show: a loading or gone one would open an empty frame.
    ...(descriptor.card === null || descriptor.title._tag === 'Pending' || descriptor.title._tag === 'Gone' ? {} : { hoverCard: <EntityHoverCard entity={descriptor.card} /> }),
    // A first load, with nothing seen before, shimmers its kind word; a remembered title shows plainly.
    ...(descriptor.title._tag === 'Pending' && seen === undefined && tab.title === undefined ? { loading: true } : {}),
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
