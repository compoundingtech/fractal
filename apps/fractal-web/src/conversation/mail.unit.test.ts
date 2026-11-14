import { readFileSync } from 'node:fs'
import { TimelineEntry } from '@fractal/client/schema'
import { Schema } from 'effect'
import { describe, expect, it } from 'vitest'
import { LiveTimeline } from './fromTimeline.ts'
import { mailOrigin, wakeExcerpt, wakeExcerptMax, wakeTitle, wakeTitleMax } from './mail.ts'
import type { ConversationItem } from './model.ts'

const fixture = (name: string): unknown => JSON.parse(readFileSync(new URL(`../../../../fixtures/clients/transcripts/${name}`, import.meta.url), 'utf8'))
type RawEntry = { id: string; sequence: number; type: string; role: string; timestamp: string; body: Record<string, unknown> }
const raw = () => fixture('wakes.json') as RawEntry[]
const project = (entries: unknown) => {
  const timeline = new LiveTimeline()
  timeline.apply({ replace: true, hasMore: false, entries: Schema.decodeUnknownSync(Schema.Array(TimelineEntry))(entries) })
  return timeline.project().items
}
const P = 'timeline-entry/session-wake/'

/** The fixture's row shape (`fixtures/clients/transcripts/*.expected.json`), read from the web's items. */
const rowOf = (item: ConversationItem) => {
  switch (item._tag) {
    case 'Event': return item.notice?._tag === 'Delivery' ? { kind: 'event', text: item.notice.subject ?? item.title } : { kind: 'event', text: item.title }
    case 'Message': return { kind: 'mail', subject: item.title ?? '', body: item.text ?? '' }
    case 'Text': return { kind: item.role === 'assistant' ? 'assistant' : 'user', text: item.text }
    default: return { kind: item._tag }
  }
}
type ExpectedRow = { id: string; body: { kind: string; value: string | { subject: string; body: string } } }
const expectedRow = ({ body: { kind, value } }: ExpectedRow) =>
  typeof value === 'string' ? { kind, text: value } : { kind, subject: value.subject, body: value.body }

describe('wakes contract (fixtures/clients/transcripts/wakes)', () => {
  it('reads the rows the shared fixture expects: two wakes as events, agent mail as one object', () => {
    const expected = fixture('wakes.expected.json') as ExpectedRow[]
    expect(project(raw()).map(rowOf)).toEqual(expected.map(expectedRow))
  })
})

describe('machine mail', () => {
  it('takes the writer from author, else from; a recipient without a writer is machine', () => {
    expect(mailOrigin({ author: 'daemon/runtime', to: 'agent/a' })).toBe('machine')
    expect(mailOrigin({ to: 'agent/a' })).toBe('machine')
    expect(mailOrigin({ author: 'person/example', from: 'person/example', to: 'agent/a' })).toBe('person')
    expect(mailOrigin({ from: 'person/example', to: 'agent/a' })).toBe('person')
    expect(mailOrigin({ from: 'agent/b', to: 'agent/a' })).toBe('agent')
  })
  it('names the trigger and step, bounded to one line', () => {
    expect(wakeTitle('Mission step ready: Review the key audit')).toBe('Woken by mission step · Review the key audit')
    expect(wakeTitle('Prepare for restart')).toBe('Woken by restart notice')
    expect(wakeTitle('Session restarted: where you were')).toBe('Woken by restart handoff')
    expect(wakeTitle(undefined)).toBe('Woken by Fractal')
    expect(wakeTitle('Disk\nfull')).toBe('Woken by Fractal · Disk full')
    expect([...wakeTitle(`Mission step ready: ${'x'.repeat(400)}`)].length).toBe(wakeTitleMax)
    expect(wakeExcerpt('\n\nFirst   line\nsecond')).toBe('First line')
    expect([...wakeExcerpt('y'.repeat(400))].length).toBe(wakeExcerptMax)
  })
  it('draws a negotiated wake as one quiet row at its body, the whole body behind Read message', () => {
    const items = project(raw())
    const wake = items.find(item => item.id === `${P}0a0a0a0a0a0a0a0a-content`)
    expect(wake).toMatchObject({ _tag: 'Event', kind: 'delivery', title: 'Woken by mission step · Review the key audit', notice: { _tag: 'Delivery', from: 'Fractal', subject: 'Mission step ready: Review the key audit' } })
    expect(wake?._tag === 'Event' && wake.notice?.payload).toContain('Wake: automatic attempt 1 (ready)')
    expect(wake?._tag === 'Event' && wake.text).toBe('A mission step is ready: step/example/review. Run `st work claim step/example/review --as agent/example/quay` to read and claim it.')
    expect(items.map(item => item.id)).not.toContain(`${P}0a0a0a0a0a0a0a0a-message`)
  })
  it('draws an unnegotiated wake (no author, no from) the same way, never as the person', () => {
    const items = project(raw())
    expect(items.find(item => item.id === `${P}1b1b1b1b1b1b1b1b-content`)).toMatchObject({ _tag: 'Event', kind: 'delivery', title: 'Woken by restart handoff' })
    expect(items.some(item => item._tag === 'Text' && item.role === 'user')).toBe(false)
  })
  it('keeps a wake header whose body is not loaded as its row, saying so', () => {
    const items = project(raw().filter(entry => entry.id !== `${P}0a0a0a0a0a0a0a0a-content`))
    expect(items.find(item => item.id === `${P}0a0a0a0a0a0a0a0a-message`)).toMatchObject({ _tag: 'Event', kind: 'delivery', text: 'Message body not loaded', notice: { payload: '' } })
  })
})

describe('mail as one object', () => {
  it('joins an agent message header and its body into one Message at the body', () => {
    const items = project(raw())
    const mail = items.find(item => item.id === `${P}2c2c2c2c2c2c2c2c-content`)
    expect(mail).toMatchObject({ _tag: 'Message', messageId: 'message/mail03', author: 'agent/example/harbor', from: 'agent/example/harbor', to: 'agent/example/quay', title: 'Key audit', text: 'Keys rotated. The billing reader still uses the old key.' })
    expect(items.map(item => item.id)).not.toContain(`${P}2c2c2c2c2c2c2c2c-message`)
  })
  it('keeps a person\'s mail as the person\'s row, with no second header row', () => {
    const entries = raw().map(entry => entry.id === `${P}2c2c2c2c2c2c2c2c-message` ? { ...entry, body: { ...entry.body, author: 'person/ada', from: 'person/ada' } } : entry)
    const items = project(entries)
    expect(items.find(item => item.id === `${P}2c2c2c2c2c2c2c2c-content`)).toMatchObject({ _tag: 'Text', role: 'user', text: 'Keys rotated. The billing reader still uses the old key.' })
    expect(items.some(item => item._tag === 'Message')).toBe(false)
  })
  it('keeps an agent header whose body is not loaded as a header-only Message', () => {
    const items = project(raw().filter(entry => entry.id !== `${P}2c2c2c2c2c2c2c2c-content`))
    const mail = items.find(item => item.id === `${P}2c2c2c2c2c2c2c2c-message`)
    expect(mail).toMatchObject({ _tag: 'Message', title: 'Key audit' })
    expect(mail?._tag === 'Message' && mail.text).toBeUndefined()
  })
  it('reads mail content whose header left the window as a message from an unknown sender, not the person', () => {
    const items = project(raw().filter(entry => entry.id !== `${P}2c2c2c2c2c2c2c2c-message`))
    const mail = items.find(item => item.id === `${P}2c2c2c2c2c2c2c2c-content`)
    expect(mail).toMatchObject({ _tag: 'Message', text: 'Keys rotated. The billing reader still uses the old key.' })
    expect(mail?._tag === 'Message' && mail.from).toBeUndefined()
  })
  it('keeps the header\'s attachments on the joined Message', () => {
    const entries = raw().map(entry => entry.id === `${P}2c2c2c2c2c2c2c2c-message`
      ? { ...entry, body: { ...entry.body, attachments: [{ blob: `blob/${'a'.repeat(64)}`, sha256: 'a'.repeat(64), media_type: 'image/png', name: 'audit.png', size: 10, origin: 'message/mail03' }] } } : entry)
    const mail = project(entries).find(item => item.id === `${P}2c2c2c2c2c2c2c2c-content`)
    expect(mail?._tag === 'Message' && mail.attachments).toEqual([{ id: `blob/${'a'.repeat(64)}`, mediaType: 'image/png', name: 'audit.png', size: 10, messageId: 'message/mail03' }])
  })
  it('joins a body that arrives in a later frame and drops the header row it replaces', () => {
    const timeline = new LiveTimeline()
    const decode = (entries: unknown) => Schema.decodeUnknownSync(Schema.Array(TimelineEntry))(entries)
    const all = raw()
    timeline.apply({ replace: true, hasMore: false, entries: decode(all.filter(entry => entry.id !== `${P}2c2c2c2c2c2c2c2c-content`)) })
    expect(timeline.project().items.map(item => item.id)).toContain(`${P}2c2c2c2c2c2c2c2c-message`)
    timeline.apply({ replace: false, hasMore: false, entries: decode(all.filter(entry => entry.id === `${P}2c2c2c2c2c2c2c2c-content`)) })
    const ids = timeline.project().items.map(item => item.id)
    expect(ids).not.toContain(`${P}2c2c2c2c2c2c2c2c-message`)
    expect(ids).toContain(`${P}2c2c2c2c2c2c2c2c-content`)
  })
  it('keeps a header row when its body projects to something other than the message', () => {
    const entries = raw().map(entry => entry.id === `${P}2c2c2c2c2c2c2c2c-content`
      ? { ...entry, body: { ...entry.body, blocks: [{ id: 'mail/hidden', kind: 'text', source_type: 'content', visibility: 'internal', payload: { body_ref: true } }] } } : entry)
    const items = project(entries)
    expect(items.find(item => item.id === `${P}2c2c2c2c2c2c2c2c-message`)).toMatchObject({ _tag: 'Message', title: 'Key audit' })
    expect(items.find(item => item.id === `${P}2c2c2c2c2c2c2c2c-content`)).toMatchObject({ _tag: 'UnknownEvent' })
  })
  it('keeps a native turn header with an author but no graph message id a turn header', () => {
    const header = { id: `${P}native-turn`, sequence: 20, revision: 1, timestamp: '2026-11-07T09:00:00.000Z', role: 'assistant', final: true, type: 'message', body: { message_id: 'native-turn-20', author: 'agent/example/quay' } }
    const body = { id: `${P}native-turn-content`, sequence: 21, revision: 1, timestamp: '2026-11-07T09:00:00.000Z', role: 'assistant', final: true, type: 'content', body: { media_type: 'text/plain', text: 'Own words.' } }
    const items = project([...raw(), header, body])
    expect(items.map(item => item.id)).not.toContain(`${P}native-turn`)
    expect(items.find(item => item.id === `${P}native-turn-content`)).toMatchObject({ _tag: 'Text', role: 'assistant', text: 'Own words.' })
  })
})
