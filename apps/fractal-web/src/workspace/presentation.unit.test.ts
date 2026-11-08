import type { Attention } from '@fractal/client/schema'
import { describe as group, expect, it, vi } from 'vitest'
import type { Agent } from '../data/source.ts'
import { workspaceKinds } from './kinds.ts'
import { corePresenters } from './present.ts'
import type { KindDescriptor, PresentationWorld } from './presentation.ts'
import type { KindContribution } from './registry.ts'
import { paneKey, subjectOfRef, type Subject, type SubjectKind } from './subject.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const agent = { ref: 'agent/example/complexity', terminal: '', name: 'Global complexity', lifecycle: { _tag: 'Unknown' }, host: 'build-1', connected: true, activity: 'idle', status: 'idle', usage: { _tag: 'Unknown' }, checkout: { _tag: 'Unknown' }, workspace: { _tag: 'Unknown' }, startedAt: { _tag: 'Unknown' }, endedAt: { _tag: 'Unknown' }, blockedOn: { _tag: 'Unknown' }, ask: { _tag: 'Unknown' }, lastActivityAt: { _tag: 'Unknown' } } as unknown as Agent
const question = {
  id: 'attention/9299a37c', title: 'Q24: Complexity skill interview', attention_kind: 'agent-request', state: 'open', actions: ['work.done'],
  requester_id: agent.ref, source_id: agent.ref, detail: '',
} as unknown as Attention

const world: PresentationWorld = {
  agents: [agent],
  rosterKnown: true,
  needsYou: new Set(),
  attention: { _tag: 'Observed', value: [question], freshness: 'live' },
  subjects: new Map([
    ['mission/7f3a', { title: 'Ship the splits' }],
    ['doc/plans/2026-q4.md', { title: 'Q4 plan' }],
    ['resource/observed/9c1e', { title: 'Build log' }],
  ]),
  missionsKnown: true,
}

/** One subject per core kind. A kind added to `Subject` needs a row here, and so a proof of its name. */
const examples: { readonly [K in SubjectKind]: Extract<Subject, { _tag: K }> } = {
  Conversation: { _tag: 'Conversation', agent: agent.ref },
  Terminal: { _tag: 'Terminal', agent: agent.ref },
  Inbox: { _tag: 'Inbox' },
  Alert: { _tag: 'Alert', id: question.id },
  Missions: { _tag: 'Missions' },
  Mission: { _tag: 'Mission', ref: 'mission/7f3a' },
  Usage: { _tag: 'Usage' },
  Document: { _tag: 'Document', ref: 'doc/plans/2026-q4.md' },
  Declaration: { _tag: 'Declaration', ref: agent.ref },
  Resource: { _tag: 'Resource', ref: 'resource/observed/9c1e' },
  Unsupported: { _tag: 'Unsupported', key: 'list:fleet' },
}

/** Kinds that have no hover card by design. Every other kind must name one. */
const noCard: ReadonlySet<SubjectKind> = new Set(['Inbox', 'Missions', 'Usage', 'Document', 'Resource', 'Unsupported'])

const describe = (subject: Subject, at: PresentationWorld = world): KindDescriptor => workspaceKinds.kindOf(subject).present(subject, at)
const lastSegment = (subject: Subject) => paneKey(subject).split(/[/:]/u).at(-1)

group('every kind names its subject', () => {
  for (const subject of Object.values(examples)) {
    it(`${subject._tag}: a real name, never its kind word or a ref fragment`, () => {
      const descriptor = describe(subject)
      if (subject._tag === 'Unsupported') return expect(descriptor.title._tag).toBe('Gone')
      expect(['Known', 'Fixed']).toContain(descriptor.title._tag)
      if (descriptor.title._tag !== 'Known') return
      expect(descriptor.title.text).not.toBe(descriptor.kind)
      expect(descriptor.title.text).not.toBe(lastSegment(subject))
    })
    it(`${subject._tag}: ${noCard.has(subject._tag) ? 'states it has no card' : 'names its hover card'}`, () => {
      expect(describe(subject).card === null).toBe(noCard.has(subject._tag))
    })
  }

  it('names a GitHub item by its handle before its title is observed, and by its title after', () => {
    const pr = subjectOfRef('resource/github/acme/garden/pull-request/7')
    expect(describe(pr).title).toEqual({ _tag: 'Known', text: 'acme/garden#7' })
    expect(describe(pr).card).toEqual({ kind: 'resource', id: 'resource/github/acme/garden/pull-request/7' })
    const titled = { ...world, subjects: new Map([['resource/github/acme/garden/pull-request/7', { title: 'Split panes' }]]) }
    expect(describe(pr, titled).title).toEqual({ _tag: 'Known', text: 'Split panes' })
  })
})

group('an alert', () => {
  it('is its question, asked by its agent, waiting on you', () => {
    const descriptor = describe(examples.Alert)
    expect(descriptor).toMatchObject({ title: { _tag: 'Known', text: 'Q24: Complexity skill interview' }, kind: 'Request', detail: 'Global complexity', card: { kind: 'alert', id: question.id } })
    expect(descriptor.mark).toEqual({ _tag: 'Status', status: { tone: 'needs-you', label: 'Waiting for you' } })
  })

  it('is pending while alerts load, and gone once answered', () => {
    expect(describe(examples.Alert, { ...world, attention: { _tag: 'Waiting' } }).title).toEqual({ _tag: 'Pending', kind: 'Alert' })
    expect(describe(examples.Alert, { ...world, attention: { _tag: 'Observed', value: [], freshness: 'live' } }).title).toEqual({ _tag: 'Gone', kind: 'Alert' })
  })
})

group('the types refuse a kind without a presenter', () => {
  it('core kinds and contributions', () => {
    // @ts-expect-error: every SubjectKind needs a presenter.
    const missing: typeof corePresenters = { ...corePresenters, Alert: undefined }
    // @ts-expect-error: a contributed kind must say how it is presented.
    const silent: KindContribution<string> = { id: 'x.y', claims: () => false, examples: ['x/1'], policy: { resources: () => [], keepWarm: false, domCost: 0, dataCost: 0 }, view: '' }
    // @ts-expect-error: a title is observed, pending or gone, never a bare string.
    const bare: KindDescriptor = { title: 'Alert', kind: 'Alert', mark: { _tag: 'Icon', name: 'alert' }, card: null }
    // @ts-expect-error: the card is stated, even when there is none.
    const unstated: KindDescriptor = { title: { _tag: 'Fixed', text: 'Inbox' }, kind: 'Inbox', mark: { _tag: 'Icon', name: 'bell' } }
    expect([missing, silent, bare, unstated]).toHaveLength(4)
  })
})
