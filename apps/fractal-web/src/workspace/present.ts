import type { IconName } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import type { ThingFace } from '../../../../packages/fractal-ui/src/assistant-ui/things/face.ts'
import { alertFrom, KIND_WORDS } from '../conversation/alerts/alertChoices.ts'
import { isWaitingForAnswer } from '../data/attention.ts'
import { agentFace } from './faces.ts'
import { Title, titleText, type KindDescriptor, type Present, type PresentationWorld } from './presentation.ts'
import type { Subject, SubjectKind } from './subject.ts'

/** Exhaustive: every core subject supplies the same face for every dense representation. */
type Presenter<K extends SubjectKind> = (subject: Extract<Subject, { _tag: K }>, world: PresentationWorld) => KindDescriptor

const nameOf = (world: PresentationWorld) => (ref: string) => world.agents.find(agent => agent.ref === ref)?.name
const face = (kind: string, icon: IconName, title: KindDescriptor['title']): ThingFace => ({ kind, icon, title: titleText(title), mark: { shape: 'none', word: '' } })

const agent = ({ agent: ref }: { readonly agent: string }, world: PresentationWorld): KindDescriptor => {
  const row = world.agents.find(each => each.ref === ref)
  const title = Title.of('Agent', world.rosterKnown, row)
  const ask = world.attention._tag === 'Observed' ? world.attention.value.find(item => (item.requester_id === ref || item.source_id === ref) && isWaitingForAnswer(item)) : undefined
  return { face: row === undefined ? face('Agent', 'person', title) : agentFace(row, ask === undefined ? undefined : { label: ask.title }), title, kind: 'Agent', mark: { _tag: 'Agent', agent: ref }, card: { kind: 'agent', id: ref } }
}

const fixed = (text: string, icon: 'bell' | 'checklist' | 'gauge', view: 'inbox' | 'missions' | 'usage'): KindDescriptor =>
  ({ face: face(text, icon, { _tag: 'Fixed', text }), title: { _tag: 'Fixed', text }, kind: text, mark: { _tag: 'Icon', name: icon }, card: { kind: 'view', id: view } })

const resource = (kind: string, icon: IconName, ref: string, title: KindDescriptor['title'], world: PresentationWorld): KindDescriptor => ({
  face: world.subjects.get(ref)?.face ?? face(kind, icon, title), title, kind, mark: { _tag: 'Icon', name: icon }, card: { kind: kind === 'Mission' ? 'mission' : 'resource', id: ref },
})

export const corePresenters: { readonly [K in SubjectKind]: Presenter<K> } = {
  Conversation: agent,
  Terminal: agent,
  Inbox: () => fixed('Inbox', 'bell', 'inbox'),
  Alert: ({ id }, world) => {
    const items = world.attention._tag === 'Observed' ? world.attention.value : undefined
    const item = items?.find(each => each.id === id)
    if (item === undefined) {
      const title = Title.of('Alert', items !== undefined, undefined)
      return { face: face('Alert', 'alert', title), title, kind: 'Alert', mark: { _tag: 'Icon', name: 'alert' }, card: { kind: 'alert', id } }
    }
    const kind = KIND_WORDS[item.attention_kind] ?? 'Alert'
    const title = Title.known(item)
    return {
      face: { ...face(kind, 'alert', title), ...(isWaitingForAnswer(item) ? { act: { tone: 'act', label: item.title } as const } : {}) },
      title, kind,
      mark: isWaitingForAnswer(item) ? { _tag: 'Status', status: { tone: 'needs-you', label: item.title } } : { _tag: 'Icon', name: 'alert' },
      card: { kind: 'alert', id }, detail: alertFrom(item, nameOf(world)),
    }
  },
  Missions: () => fixed('Missions', 'checklist', 'missions'),
  Mission: ({ ref }, world) => resource('Mission', 'checklist', ref, Title.of('Mission', world.missionsKnown, world.subjects.get(ref)), world),
  Usage: ({ group }) => group === undefined ? fixed('Usage', 'gauge', 'usage') : { ...fixed('Usage', 'gauge', 'usage'), detail: group },
  Document: ({ ref }, world) => resource('Document', 'attach', ref, Title.known(world.subjects.get(ref) ?? { name: ref.slice(ref.lastIndexOf('/') + 1) }), world),
  Declaration: ({ ref }, world) => {
    const name = nameOf(world)(ref) ?? world.subjects.get(ref)?.title
    const title = name === undefined ? Title.of('Declaration', world.rosterKnown && world.missionsKnown, undefined) : Title.known({ name: `${name} KDL` })
    return { face: face('Declaration', 'pencil', title), title, kind: 'Declaration', mark: { _tag: 'Icon', name: 'pencil' }, card: ref.startsWith('agent/') ? { kind: 'agent', id: ref } : null }
  },
  Resource: ({ ref }, world) => ({ ...resource('Resource', 'folder', ref, Title.of('Resource', true, world.subjects.get(ref)), world), detail: ref }),
  Unsupported: ({ key }) => ({ face: face('View', 'info', Title.gone('View')), title: Title.gone('View'), kind: 'View', mark: { _tag: 'Icon', name: 'info' }, card: null, detail: key }),
}

export const presentCore: Present = (subject, world) => (corePresenters[subject._tag] as Present)(subject, world)
