import { Id } from '@fractal/client/schema/read'
import { Schema } from 'effect'

/** Local presentation identity, persisted as part of the shell's navigation state. */
export const PresentationId = Schema.Literals(['detail', 'overview', 'resources']).annotate({
  identifier: 'Wf.Subject.PresentationId',
})
export type PresentationId = typeof PresentationId.Type

/** A subject can be addressed before its first authoritative observation arrives. */
export const SubjectAddress = Schema.Struct({
  ref: Id,
  presentation: PresentationId,
}).annotate({ identifier: 'Wf.Subject.Address' })
export type SubjectAddress = typeof SubjectAddress.Type
