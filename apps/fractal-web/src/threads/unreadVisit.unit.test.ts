import * as Native from '@fractal/client/schema/read'
import { expect, it } from 'vitest'
import * as fc from 'fast-check'
import { awayVisit, enterUnreadVisit, holdUnreadVisit, planUnreadView } from './unreadVisit.ts'

const reminder = Native.decodeUnknownSync(Native.ThreadUnreadReminder)({ anchor_message_id: 'message/a', marks: [
  { mark_id: 'operation/a', anchor_message_id: 'message/a' },
  { mark_id: 'operation/b', anchor_message_id: 'message/b' },
] })
const view = { foreground: true, visibleMessageIds: ['message/a'], currentReminder: reminder }

it('clears only captured marks whose own anchor was viewed', () => {
  expect(planUnreadView(enterUnreadVisit(reminder), view).observed_marks).toEqual(['operation/a'])
})
it('is sticky after an explicit mark until a later deliberate visit', () => {
  expect(planUnreadView(holdUnreadVisit(enterUnreadVisit(reminder)), view).observed_marks).toEqual([])
  expect(planUnreadView(enterUnreadVisit(reminder), view).observed_marks).toEqual(['operation/a'])
})
it('does not count away or background views and does not consume them', () => {
  expect(planUnreadView(awayVisit, view).observed_marks).toEqual([])
  const visit = enterUnreadVisit(reminder)
  expect(planUnreadView(visit, { ...view, foreground: false })).toEqual({ visit, observed_marks: [] })
})
it('never captures marks arriving during a visit, including an initially unknown reminder', () => {
  const later = Native.decodeUnknownSync(Native.ThreadUnreadReminder)({ ...reminder, marks: [...reminder.marks, { mark_id: 'operation/concurrent', anchor_message_id: 'message/a' }] })
  expect(planUnreadView(enterUnreadVisit(reminder), { ...view, currentReminder: later }).observed_marks).toEqual(['operation/a'])
  expect(planUnreadView(enterUnreadVisit(undefined), view).observed_marks).toEqual([])
})
it('attempts each captured ID once per visit and never clears removed IDs', () => {
  const first = planUnreadView(enterUnreadVisit(reminder), view)
  expect(planUnreadView(first.visit, view).observed_marks).toEqual([])
  expect(planUnreadView(enterUnreadVisit(reminder), { ...view, currentReminder: undefined }).observed_marks).toEqual([])
})
it('captures immutable marks rather than retaining a mutable summary', () => {
  const anchor = Native.decodeUnknownSync(Native.Id)('message/a')
  const input = { anchor_message_id: anchor, marks: [{ mark_id: 'id', anchor_message_id: anchor }] }
  const visit = enterUnreadVisit(input)
  input.marks[0]!.anchor_message_id = Native.decodeUnknownSync(Native.Id)('message/b')
  expect(planUnreadView(visit, { foreground: true, visibleMessageIds: ['message/b'], currentReminder: input }).observed_marks).toEqual([])
})
it('preserves every unobserved concurrent mark for arbitrary opaque IDs and anchors', () => {
  fc.assert(fc.property(fc.uniqueArray(fc.string(), { minLength: 1 }), fc.array(fc.boolean()), (ids, selected) => {
    const marks = ids.map((mark_id, index) => Native.decodeUnknownSync(Native.ThreadUnreadMark)({ mark_id, anchor_message_id: `anchor/${index}` }))
    const visible = [...marks.filter((_, index) => selected[index % Math.max(1, selected.length)]).map(mark => mark.anchor_message_id), 'message/concurrent']
    const captured = Native.decodeUnknownSync(Native.ThreadUnreadReminder)({ anchor_message_id: marks[0]!.anchor_message_id, marks })
    const concurrent = Native.decodeUnknownSync(Native.ThreadUnreadReminder)({ ...captured, marks: [...marks, { mark_id: `${ids.join('')}!`, anchor_message_id: 'message/concurrent' }] })
    const result = planUnreadView(enterUnreadVisit(captured), { foreground: true, visibleMessageIds: visible, currentReminder: concurrent })
    expect(result.observed_marks).toEqual(marks.filter(mark => visible.includes(mark.anchor_message_id)).map(mark => mark.mark_id))
  }))
})
