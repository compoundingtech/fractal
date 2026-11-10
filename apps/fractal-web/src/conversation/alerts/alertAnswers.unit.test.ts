import { Attention, decodeUnknownSync } from '@fractal/client/schema'
import { describe, expect, it } from 'vitest'
import { createAlertAnswers, type AnswerResult } from './alertAnswers.ts'

const alert = (attention_kind: Attention['attention_kind'], fields: Record<string, unknown> = {}) => decodeUnknownSync(Attention)({
  id: 'attention/decision', kind: 'attention', revision: 'r1', episode: 'episode/one', state: 'open', attention_kind,
  title: 'Approve the change?', detail: 'A synthetic decision', priority: 'normal', actions: [],
  person_id: 'person/operator', source_id: 'agent/example',
  updated_at: '2026-11-01T12:00:00Z', requested_at: '2026-11-01T12:00:00Z',
  ...fields,
})

describe('answer visibility across authoritative revisions', () => {
  it('keeps a sent non-prompt answer hidden only on the revision that was answered', async () => {
    const answers = createAlertAnswers()
    const item = alert('person-step')
    answers.observe([item])
    answers.send(item, 'Approve', async () => ({ _tag: 'Sent' }))
    await Promise.resolve()
    expect(answers.getSnapshot().pending.get(item.id)?.phase).toBe('sent')
    expect(answers.hidden(item)).toBe(true)

    const raisedAgain = alert(item.attention_kind, { revision: 'r2' })
    answers.observe([raisedAgain])
    expect(answers.hidden(raisedAgain)).toBe(false)
    expect(answers.hidden(item)).toBe(true)
  })

  it('keeps a harness answer hidden across revisions of the same episode, but not a new prompt', async () => {
    const answers = createAlertAnswers()
    const item = alert('harness-prompt')
    answers.observe([item])
    answers.send(item, 'Allow', async () => ({ _tag: 'Sent' }))
    await Promise.resolve()
    expect(answers.hidden(alert(item.attention_kind, { revision: 'r2' }))).toBe(true)
    expect(answers.hidden(alert(item.attention_kind, { revision: 'r2', episode: 'episode/two' }))).toBe(false)
  })
})

type Completion = 'sent' | 'refused' | 'rejected'
const completions: readonly Completion[] = ['sent', 'refused', 'rejected']
const inFlight = () => {
  const { promise, resolve, reject } = Promise.withResolvers<AnswerResult>()
  return {
    promise,
    complete: async (result: Completion, detail: string, settled = false) => {
      if (result === 'rejected') reject(new Error(detail))
      else resolve(result === 'sent' ? { _tag: 'Sent', settled } : { _tag: 'Refused', detail })
      await Promise.resolve()
    },
  }
}

describe.each(['older-first', 'newer-first'] as const)('overlapping revision answers: %s', (order) => {
  for (const olderResult of completions) {
    it.each(completions)(`ignores older ${olderResult} completion when the newer answer is %s`, async (newerResult) => {
      const answers = createAlertAnswers()
      const older = alert('person-step')
      const newer = alert(older.attention_kind, { revision: 'r2' })
      const first = inFlight()
      const second = inFlight()
      answers.observe([older])
      answers.send(older, 'Old choice', () => first.promise)
      answers.observe([newer])
      answers.send(newer, 'New choice', () => second.promise)
      const sending = answers.getSnapshot()
      expect(sending.pending.get(newer.id)).toMatchObject({ item: newer, label: 'New choice', phase: 'sending' })

      if (order === 'older-first') {
        // Even an old source-settled success must not settle, advance, or roll back the newer answer.
        await first.complete(olderResult, 'Old refusal', true)
        expect(answers.getSnapshot()).toBe(sending)
        expect(answers.hidden(newer)).toBe(true)
        expect(answers.failure(newer)).toBeUndefined()
        expect(answers.getSnapshot().flash).toBeUndefined()
        expect(answers.getSnapshot().settled.has(newer.id)).toBe(false)
        await second.complete(newerResult, 'New refusal')
      } else {
        await second.complete(newerResult, 'New refusal')
        const completed = answers.getSnapshot()
        await first.complete(olderResult, 'Old refusal', true)
        expect(answers.getSnapshot()).toBe(completed)
      }

      if (newerResult === 'sent') {
        expect(answers.getSnapshot().pending.get(newer.id)).toMatchObject({ item: newer, phase: 'sent' })
        expect(answers.hidden(newer)).toBe(true)
        expect(answers.failure(newer)).toBeUndefined()
        expect(answers.getSnapshot().flash).toBeUndefined()
      } else {
        expect(answers.getSnapshot().pending.has(newer.id)).toBe(false)
        expect(answers.hidden(newer)).toBe(false)
        const detail = newerResult === 'refused' ? 'New refusal'
          : 'The answer could not be confirmed; check the alert before answering again. It was not sent again.'
        expect(answers.failure(newer)).toBe(detail)
        expect(answers.failure(older)).toBeUndefined()
        expect(answers.getSnapshot().flash?.detail).toBe(detail)
      }
      expect(answers.getSnapshot().settled.has(newer.id)).toBe(false)
    })
  }
})
