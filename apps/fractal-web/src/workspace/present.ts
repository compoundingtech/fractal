import { alertFrom, KIND_WORDS } from '../conversation/alerts/alertChoices.ts'
import { isWaitingForAnswer } from '../data/attention.ts'
import { Title, type KindDescriptor, type Present, type PresentationWorld } from './presentation.ts'
import type { Subject, SubjectKind } from './subject.ts'

/**
 * Each core kind's descriptor, over the observations every surface already reads. Exhaustive by type: a kind
 * added to `Subject` without a presenter here does not compile.
 */
type Presenter<K extends SubjectKind> = (subject: Extract<Subject, { _tag: K }>, world: PresentationWorld) => KindDescriptor

const nameOf = (world: PresentationWorld) => (ref: string) => world.agents.find(agent => agent.ref === ref)?.name

const agent = ({ agent: ref }: { readonly agent: string }, world: PresentationWorld): KindDescriptor => {
  const row = world.agents.find(each => each.ref === ref)
  return { title: Title.of('Agent', world.rosterKnown, row), kind: 'Agent', mark: { _tag: 'Agent', agent: ref }, card: { kind: 'agent', id: ref } }
}

/** A list or screen: named by its kind, by design, with a card summarising what it lists. */
const fixed = (text: string, icon: 'bell' | 'checklist' | 'gauge', view: 'inbox' | 'missions' | 'usage'): KindDescriptor =>
  ({ title: { _tag: 'Fixed', text }, kind: text, mark: { _tag: 'Icon', name: icon }, card: { kind: 'view', id: view } })

export const corePresenters: { readonly [K in SubjectKind]: Presenter<K> } = {
  Conversation: agent,
  Terminal: agent,
  Inbox: () => fixed('Inbox', 'bell', 'inbox'),
  Alert: ({ id }, world) => {
    const items = world.attention._tag === 'Observed' ? world.attention.value : undefined
    const item = items?.find(each => each.id === id)
    if (item === undefined) return { title: Title.of('Alert', items !== undefined, undefined), kind: 'Alert', mark: { _tag: 'Icon', name: 'alert' }, card: { kind: 'alert', id } }
    const kind = KIND_WORDS[item.attention_kind] ?? 'Alert'
    return {
      title: Title.known(item),
      kind,
      mark: isWaitingForAnswer(item) ? { _tag: 'Status', status: { tone: 'needs-you', label: 'Waiting for you' } } : { _tag: 'Icon', name: 'alert' },
      card: { kind: 'alert', id },
      detail: alertFrom(item, nameOf(world)),
    }
  },
  Missions: () => fixed('Missions', 'checklist', 'missions'),
  Mission: ({ ref }, world) => ({ title: Title.of('Mission', world.missionsKnown, world.subjects.get(ref)), kind: 'Mission', mark: { _tag: 'Icon', name: 'checklist' }, card: { kind: 'mission', id: ref } }),
  Usage: ({ group }) => group === undefined ? fixed('Usage', 'gauge', 'usage') : { ...fixed('Usage', 'gauge', 'usage'), detail: group },
  // A document's ref ends in its name, which is what the person called it.
  Document: ({ ref }, world) => ({ title: Title.known(world.subjects.get(ref) ?? { name: ref.slice(ref.lastIndexOf('/') + 1) }), kind: 'Document', mark: { _tag: 'Icon', name: 'attach' }, card: { kind: 'resource', id: ref } }),
  Declaration: ({ ref }, world) => {
    const name = nameOf(world)(ref) ?? world.subjects.get(ref)?.title
    return { title: name === undefined ? Title.of('Declaration', world.rosterKnown && world.missionsKnown, undefined) : Title.known({ name: `${name} KDL` }), kind: 'Declaration', mark: { _tag: 'Icon', name: 'pencil' }, card: ref.startsWith('agent/') ? { kind: 'agent', id: ref } : null }
  },
  Resource: ({ ref }, world) => ({ title: Title.of('Resource', true, world.subjects.get(ref)), kind: 'Resource', mark: { _tag: 'Icon', name: 'folder' }, card: { kind: 'resource', id: ref }, detail: ref }),
  Unsupported: ({ key }) => ({ title: Title.gone('View'), kind: 'View', mark: { _tag: 'Icon', name: 'info' }, card: null, detail: key }),
}

/** The core presenter for a subject. Contributed kinds bring their own through the registry. */
export const presentCore: Present = (subject, world) => (corePresenters[subject._tag] as Present)(subject, world)
