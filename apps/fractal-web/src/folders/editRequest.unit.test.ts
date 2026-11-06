import { describe, expect, it } from 'vitest'
import type { EditOutcome } from './edit.ts'
import { prepareArrangementRetry, stageArrangementEdit, type EditJob } from './editRequest.ts'
import { testArrangement, testSelection } from './testGateway.ts'

const job = (): EditJob => ({
  id: 'action/example', key: 'edit/example', done: Promise.withResolvers<EditOutcome>(),
  operations: [{ op: 'folder.rename', id: '00000000-0000-7000-8000-000000000002', name: 'Renamed' }],
})

describe('authoritative folder request staging', () => {
  it('replays uncertain delivery with the original identity and parameters, refreshing only the fence', () => {
    const edit = job()
    const winner = testArrangement()
    const first = stageArrangementEdit(edit, testSelection.owner, winner, 'arrangement/reserved', 'snapshot/first', false)
    expect(first._tag).toBe('Ready')
    const original = structuredClone(edit.request)
    const next = stageArrangementEdit(edit, testSelection.owner, { ...winner, revision: 'claim/new' }, 'arrangement/reserved', 'snapshot/next', false)
    expect(next).toEqual({ _tag: 'Ready', request: { ...original, fence: { snapshot_id: 'snapshot/next', subject_revisions: { [winner.id]: 'claim/new' } } } })
    expect(prepareArrangementRetry(edit, winner, 'arrangement/reserved')).toBe(edit)
  })

  it('changes identity only when a definitive refusal requires different parameters', () => {
    const edit = job()
    stageArrangementEdit(edit, testSelection.owner, undefined, 'arrangement/reserved', 'snapshot/first', false)
    const retried = prepareArrangementRetry(edit, testArrangement(), 'arrangement/reserved')
    expect(retried.id).not.toBe(edit.id)
    expect(retried.key).not.toBe(edit.key)
    expect(retried.request).toBeUndefined()
    expect(stageArrangementEdit(retried, testSelection.owner, testArrangement(), 'arrangement/reserved', 'snapshot/next', false)).toMatchObject({
      _tag: 'Ready', request: { parameters: { subject: testSelection.subject, operations: edit.operations } },
    })
  })

  it('does not restage structural intent against a different Sidebar', () => {
    let restaged = false
    const edit: EditJob = { ...job(), sidebar: 'arrangement/original', intent: { restage: () => { restaged = true; return { _tag: 'Satisfied' } } } }
    expect(stageArrangementEdit(edit, testSelection.owner, testArrangement(), 'arrangement/reserved', 'snapshot/next', false)).toEqual({
      _tag: 'Refused', detail: 'The folder layout changed elsewhere; make the change again.', restageRefusal: 'The folder layout changed elsewhere; make the change again.',
    })
    expect(restaged).toBe(false)
    expect(edit.request).toBeUndefined()
  })
})
