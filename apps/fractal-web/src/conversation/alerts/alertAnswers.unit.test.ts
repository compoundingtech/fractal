import type * as Native from '@fractal/client/schema'
import { describe, expect, it } from 'vitest'
import { createAlertAnswers } from './alertAnswers.ts'

const alert = (attention_kind: Native.Attention['attention_kind']) => ({
  id: 'attention/decision', revision: 'r1', episode: 'episode/one', state: 'open', attention_kind,
}) as Native.Attention

describe('answer visibility across authoritative revisions', () => {
  it('keeps a sent non-prompt answer hidden only on the revision that was answered', async () => {
    const answers = createAlertAnswers()
    const item = alert('person-step')
    answers.observe([item])
    answers.send(item, 'Approve', async () => ({ _tag: 'Sent' }))
    await Promise.resolve()
    expect(answers.getSnapshot().pending.get(item.id)?.phase).toBe('sent')
    expect(answers.hidden(item)).toBe(true)

    const raisedAgain = { ...item, revision: 'r2' }
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
    expect(answers.hidden({ ...item, revision: 'r2' })).toBe(true)
    expect(answers.hidden({ ...item, revision: 'r2', episode: 'episode/two' })).toBe(false)
  })
})
