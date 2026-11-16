import { Schema } from 'effect'

/**
 * What a tab shows. The persisted identity is the TUI's opaque pane key (`crates/fractal-tui/src/ui/pane.rs`),
 * which is also what a glass stores, so one person's tabs can mean the same thing in every client.
 * A key this client cannot render is kept verbatim as `Unsupported`: dropping it would delete another
 * client's tab on the next sync.
 */
export const Subject = Schema.TaggedUnion({
  Conversation: { agent: Schema.String },
  /** Only as a key or a link: a tab this client opens is the agent's, with its terminal face shown. */
  Terminal: { agent: Schema.String },
  Inbox: {},
  Alert: { id: Schema.String },
  Missions: {},
  Mission: { ref: Schema.String },
  Usage: { group: Schema.optionalKey(Schema.String) },
  Document: { ref: Schema.String },
  /** A subject's whole declaration, as KDL. */
  Declaration: { ref: Schema.String },
  /** Any other ref: a generic resource tab, unless a contributed kind claims it. */
  Resource: { ref: Schema.String },
  Unsupported: { key: Schema.String },
})
export type Subject = typeof Subject.Type

export type SubjectKind = Subject['_tag']

export const paneKey: (subject: Subject) => string = Subject.match({
  Conversation: ({ agent }) => `agent:${agent}`,
  Terminal: ({ agent }) => `terminal:${agent}`,
  Inbox: () => 'list:home',
  Alert: ({ id }) => `home:${id}`,
  Missions: () => 'list:missions',
  Mission: ({ ref }) => `mission:${ref}`,
  Usage: ({ group }) => `usage:${group ?? ''}`,
  Document: ({ ref }) => `document:${ref}`,
  Declaration: ({ ref }) => `declaration:${ref}`,
  Resource: ({ ref }) => `resource:${ref}`,
  Unsupported: ({ key }) => key,
})

export const parsePane = (key: string): Subject => {
  const separator = key.indexOf(':')
  const unsupported: Subject = { _tag: 'Unsupported', key }
  if (separator <= 0) return unsupported
  const kind = key.slice(0, separator), value = key.slice(separator + 1)
  switch (kind) {
    case 'agent': return value === '' ? unsupported : { _tag: 'Conversation', agent: value }
    case 'terminal': return value.startsWith('agent/') ? { _tag: 'Terminal', agent: value } : unsupported
    case 'list': return value === 'home' ? { _tag: 'Inbox' } : value === 'missions' ? { _tag: 'Missions' } : unsupported
    case 'home': return value === '' ? unsupported : { _tag: 'Alert', id: value }
    case 'mission': return value === '' ? unsupported : { _tag: 'Mission', ref: value }
    case 'usage': return value === '' ? { _tag: 'Usage' } : { _tag: 'Usage', group: value }
    case 'document': return value === '' ? unsupported : { _tag: 'Document', ref: value }
    case 'declaration': return value === '' ? unsupported : { _tag: 'Declaration', ref: value }
    case 'resource': return value === '' ? unsupported : { _tag: 'Resource', ref: value }
    default: return unsupported
  }
}

const agentTerminalPrefix = 'terminal:agent/'

/** Whether a key names an agent's terminal, which is a face of that agent's one tab. */
export const isAgentTerminalPane = (key: string): boolean => key.startsWith(agentTerminalPrefix)

/** What makes two keys one tab: an agent and its terminal share theirs. Every other key is its own. */
export const paneIdentity = (key: string): string =>
  isAgentTerminalPane(key) ? `agent:${key.slice('terminal:'.length)}` : key

/** The agent whose header, details and sidebar selection follow this tab. */
export const subjectAgent = (subject: Subject | undefined): string | undefined =>
  subject?._tag === 'Conversation' || subject?._tag === 'Terminal' ? subject.agent : undefined

const encodePath = (ref: string) => ref.split('/').map(encodeURIComponent).join('/')

/** The one deep link of a subject. The URL names the active subject and nothing about the other tabs. */
export const subjectPath: (subject: Subject) => string = Subject.match({
  Conversation: ({ agent }) => `/w/${encodePath(agent)}`,
  Terminal: ({ agent }) => `/w/terminal/${encodePath(agent.slice('agent/'.length))}`,
  Inbox: () => '/w/inbox',
  Alert: ({ id }) => `/w/inbox/${encodeURIComponent(id)}`,
  Missions: () => '/w/missions',
  Mission: ({ ref }) => `/w/${encodePath(ref)}`,
  Usage: ({ group }) => group === undefined ? '/w/usage' : `/w/usage/${encodePath(group)}`,
  Document: ({ ref }) => `/w/${encodePath(ref)}`,
  Declaration: ({ ref }) => `/w/declaration/${encodePath(ref)}`,
  Resource: subject => `/w/pane/${encodeURIComponent(paneKey(subject))}`,
  Unsupported: subject => `/w/pane/${encodeURIComponent(paneKey(subject))}`,
})

/** The one ref of a subject: what `open(ref)` and a drag carry. `subjectOfRef` reads it back. */
export const subjectRef: (subject: Subject) => string = Subject.match({
  Conversation: ({ agent }) => agent,
  Terminal: ({ agent }) => `terminal/${agent.slice('agent/'.length)}`,
  Inbox: () => 'inbox',
  Alert: ({ id }) => `alert/${id}`,
  Missions: () => 'missions',
  Mission: ({ ref }) => ref,
  Usage: ({ group }) => group === undefined ? 'monitor/quota' : `usage/${group}`,
  Document: ({ ref }) => ref,
  Declaration: ({ ref }) => `declaration/${ref}`,
  Resource: subject => `pane/${paneKey(subject)}`,
  Unsupported: subject => `pane/${paneKey(subject)}`,
})

/** A subject ref as the rest of the app names it (`OpenRequest`, sidebar rows, document links). */
export const subjectOfRef = (ref: string): Subject => {
  if (ref === 'inbox') return { _tag: 'Inbox' }
  if (ref === 'missions') return { _tag: 'Missions' }
  if (ref === 'monitor/quota') return { _tag: 'Usage' }
  if (ref.startsWith('usage/')) return { _tag: 'Usage', group: ref.slice('usage/'.length) }
  if (ref.startsWith('agent/')) return { _tag: 'Conversation', agent: ref }
  if (ref.startsWith('terminal/')) return { _tag: 'Terminal', agent: `agent/${ref.slice('terminal/'.length)}` }
  if (ref.startsWith('mission/')) return { _tag: 'Mission', ref }
  if (ref.startsWith('doc/')) return { _tag: 'Document', ref }
  if (ref.startsWith('alert/')) return { _tag: 'Alert', id: ref.slice('alert/'.length) }
  if (ref.startsWith('attention/')) return { _tag: 'Alert', id: ref }
  if (ref.startsWith('declaration/')) return { _tag: 'Declaration', ref: ref.slice('declaration/'.length) }
  if (ref.startsWith('pane/')) return parsePane(ref.slice('pane/'.length))
  return { _tag: 'Resource', ref }
}

/** Parses this client's route, including `?open=` links from before tabs. `undefined` means no subject. */
export const subjectOfLocation = (location: string): Subject | undefined => {
  const [pathname = '', search = ''] = location.split('?')
  const legacy = new URLSearchParams(search).get('open')
  if (legacy !== null && legacy !== 'thread' && legacy !== 'changes') {
    const separator = legacy.lastIndexOf(':')
    return subjectOfRef(separator > 0 ? legacy.slice(0, separator) : legacy)
  }
  const encoded = pathname.match(/^\/w\/(.+)$/)?.[1]
  if (encoded === undefined) return undefined
  let path: string
  try {
    path = encoded.split('/').map(decodeURIComponent).join('/')
  } catch {
    return undefined
  }
  if (path.startsWith('inbox/')) return { _tag: 'Alert', id: path.slice('inbox/'.length) }
  if (path === 'usage') return { _tag: 'Usage' }
  return subjectOfRef(path)
}

/** Everything in the query except the subject survives navigation, such as the dev replay signal. */
export const subjectHref = (subject: Subject, search: string): string => {
  const params = new URLSearchParams(search)
  params.delete('open')
  const query = params.toString()
  return `${subjectPath(subject)}${query === '' ? '' : `?${query}`}`
}
