import type { ActionOf, Arrangement } from '@fractal/client'
import { ArrangementEditParameters } from '@fractal/client/schema'
import { Schema } from 'effect'
import type { EditOutcome, SidebarOperation, StructuralIntent } from './edit.ts'

export interface EditJob {
  operations: readonly SidebarOperation[]
  readonly intent?: StructuralIntent
  /** Sidebar selected when the gesture was accepted; undefined only for a bootstrap with no Sidebar. */
  readonly sidebar?: string
  readonly id: string
  readonly key: string
  done: PromiseWithResolvers<EditOutcome>
  request?: ActionOf<'arrangement.edit'>
  /** The last submitted request was authoritatively refused, not transport-uncertain. */
  requestRefused?: boolean
  creationObserved?: boolean
}

export const prepareArrangementRetry = (job: EditJob, winner: Arrangement | undefined, reservation: string): EditJob => {
  if (job.intent !== undefined) {
    const { request: _request, ...fields } = job
    return { ...fields, id: `action/${crypto.randomUUID()}`, key: crypto.randomUUID() }
  }
  const request = job.request
  if (request === undefined || (request.parameters.subject === (winner?.id ?? reservation) &&
    request.parameters.operations.some((operation) => operation.op === 'create') === (winner === undefined))) return job
  // Changed parameters after a known non-applied refusal need a new action identity/key.
  const { request: _request, ...fields } = job
  return { ...fields, id: `action/${crypto.randomUUID()}`, key: crypto.randomUUID() }
}

type StagedEdit =
  | { readonly _tag: 'Ready'; readonly request: ActionOf<'arrangement.edit'> }
  | { readonly _tag: 'Satisfied' }
  | { readonly _tag: 'Refused'; readonly detail: string; readonly restageRefusal?: string }

/** Stage against the fresh authoritative winner, preserving uncertain request identity. */
export const stageArrangementEdit = (job: EditJob, owner: string, winner: Arrangement | undefined, reservation: string, snapshot: string, creationAcknowledged: boolean): StagedEdit => {
  // A structural intent uses fresh siblings, but only within the Sidebar the user edited.
  // Bootstrap is the narrow exception: no Sidebar existed, so only the reserved creation target qualifies.
  if (job.request === undefined && job.intent !== undefined) {
    const bound = job.sidebar === undefined ? winner === undefined || winner.id === reservation : winner?.id === job.sidebar
    const verdict = bound ? job.intent.restage(winner) : { _tag: 'Refused' as const, sentence: 'The folder layout changed elsewhere; make the change again.' }
    if (verdict._tag === 'Refused') return { _tag: 'Refused', detail: verdict.sentence, restageRefusal: verdict.sentence }
    if (verdict._tag === 'Satisfied') return verdict
    job.operations = structuredClone(verdict.operations)
  }
  if (job.request === undefined) {
    if (winner === undefined && creationAcknowledged)
      return { _tag: 'Refused', detail: 'The created Sidebar is not visible in the complete inventory yet. Refresh before retrying.' }
    const parameters = { owner, subject: winner?.id ?? reservation, operations: winner === undefined ? [{ op: 'create', name: 'Sidebar' } as const, ...job.operations] : [...job.operations] }
    if (!Schema.is(Schema.toEncoded(ArrangementEditParameters))(parameters as unknown))
      return { _tag: 'Refused', detail: 'The edit does not match the generated arrangement operation contract.' }
    // TODO(live-migration: arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both): keep old names until all members accept aliases; sunset 60 days after first shipping release.
    job.request = { api_version: 'st3.client.v0', type: 'arrangement.edit', id: job.id, idempotency_key: job.key, parameters, fence: { snapshot_id: snapshot, subject_revisions: winner === undefined ? {} : { [winner.id]: winner.revision } } }
  } else {
    // Unknown delivery retries must replay exactly the original identity and parameters.
    const unobservedCreation = job.request.parameters.operations.some((operation) => operation.op === 'create') && !job.creationObserved
    if (winner?.id !== job.request.parameters.subject && !(winner === undefined && unobservedCreation))
      return { _tag: 'Refused', detail: 'The folder layout changed elsewhere; refresh and try again.' }
    job.request = { ...job.request, fence: { snapshot_id: snapshot, subject_revisions: winner === undefined ? {} : { [winner.id]: winner.revision } } }
  }
  return { _tag: 'Ready', request: job.request }
}
