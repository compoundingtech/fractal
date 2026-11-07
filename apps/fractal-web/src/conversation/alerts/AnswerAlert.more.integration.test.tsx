// @vitest-environment jsdom
import { act } from 'react'
import type * as React from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'
import type * as Native from '@fractal/client/schema'
import { DateTime } from 'effect'

vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  createTheme: () => ({}),
  keyframes: () => 'test-animation',
  props: () => ({}),
}))

import { AlertAnswersContext, createAlertAnswers } from './alertAnswers.ts'
import { AnswerAlert } from './AnswerAlert.tsx'

const question = 'Delete 118 archived folders on host-a (114) and host-b (4)?'
const longRef = 'resource/cache/decision/7fe01c2a9b3d4e5f60718293a4b5c6d7e8f9/record'
const alert = (request: Record<string, unknown>, title = 'Delete 118 archived folders (host-a, host-b)') => ({
  id: 'attention/more', kind: 'attention', revision: 'r1', attention_kind: 'person-step', title, detail: '', priority: 'high', state: 'open',
  requested_at: DateTime.makeUnsafe('2026-10-14T10:00:00Z'), person_id: 'person/avery', source_id: 'step-run/example/build',
  requester_id: 'agent/example/atlas', episode: 'claim/example', actions: ['work.done'],
  request: { version: 1, type: 'decision', question, why_person: 'You own the machine.', answers: [{ id: 'go', label: 'Delete', consequence: 'Gone.', outcome: 'accept' }], ...request },
}) as unknown as Native.Attention

let cleanup: (() => void) | undefined
afterEach(() => cleanup?.())

const mount = async (props: Partial<React.ComponentProps<typeof AnswerAlert>> & { readonly alert: Native.Attention }) => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  cleanup = () => { act(() => root.unmount()); container.remove() }
  await act(async () => root.render(<AlertAnswersContext.Provider value={createAlertAnswers()}><AnswerAlert onAct={async () => ({ _tag: 'Sent' })} {...props} /></AlertAnswersContext.Provider>))
  return container
}
const press = (container: HTMLElement, label: string) =>
  act(async () => [...container.querySelectorAll('button')].find((each) => each.textContent?.startsWith(label))!.click())
const part = (container: HTMLElement, label: string) => container.querySelector<HTMLElement>(`section[aria-label="${label}"]`)

it('holds its parts behind More, and swaps the clipped context line for the whole of it', async () => {
  const summary = `${'The migration moved every output last week. '.repeat(3)}\nSecond paragraph.`
  const container = await mount({ alert: alert({ summary, reasons: ['Archives verified.'] }) })
  expect(part(container, 'Evidence')).toBeNull()
  expect(container.querySelector('p')?.textContent).toContain('The migration moved')
  await press(container, 'More')
  expect(part(container, 'Context')?.textContent).toContain('Second paragraph.')
  expect(part(container, 'Why you')?.textContent).toContain('You own the machine.')
  expect(part(container, 'Why you')?.textContent).not.toContain('Why you:')
  expect([...container.querySelectorAll('section section li')].map((each) => each.textContent)).toEqual(['Archives verified.'])
})

it('does not repeat a title the heading already says, and keeps one that adds something', async () => {
  const repeated = await mount({ alert: alert({}), density: 'full' })
  expect(repeated.textContent).not.toContain('(host-a, host-b)')
  cleanup?.()
  const differing = await mount({ alert: alert({ summary: 'Short.' }, 'Quarterly storage review'), density: 'full' })
  expect(differing.textContent).toContain('Quarterly storage review')
})

it('sets raw ids apart in evidence: a long one is a chip to copy', async () => {
  const container = await mount({ alert: alert({ reasons: [`Recorded as ${longRef} after the compare.`] }), density: 'full' })
  const item = part(container, 'Evidence')!.querySelector('li')!
  expect(item.querySelector(`button[aria-label="Copy ${longRef}"]`)).not.toBeNull()
  expect(item.textContent).toContain('Recorded as ')
})

it('opens a subject as a whole row: in the app when the host can, out to a checked link, else a chip to copy', async () => {
  const onOpenRef = vi.fn()
  const container = await mount({
    density: 'full', onOpenRef, opensRef: (ref) => ref.startsWith('doc/'),
    alert: alert({ subjects: [
      { kind: 'document', label: 'Rename notes', ref: 'doc/rename-notes' },
      { kind: 'pull_request', label: 'The change', url: 'https://example.com/pull/1', ref: 'doc/ignored' },
      { kind: 'run', label: 'The run', ref: 'run/example/1' },
    ] }),
  })
  const related = part(container, 'Related')!
  expect(related.textContent).toContain('Document')
  expect(related.textContent).toContain('Pull request')
  await press(related, 'Rename notes')
  expect(onOpenRef).toHaveBeenCalledExactlyOnceWith('doc/rename-notes')
  expect(related.querySelector('a')?.getAttribute('href')).toBe('https://example.com/pull/1')
  expect(related.querySelector('button[aria-label="Copy run/example/1"]')).not.toBeNull()
  expect(related.querySelector('button[aria-label="Copy doc/ignored"]')).toBeNull()
})
