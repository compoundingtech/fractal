import { SubjectNote, decodeUnknownSync } from '@smalltalk/st3-client/schema'

/** Synthetic story fixtures: wire-shaped notes decoded through the generated client schema. */
export const notesNow = Date.parse('2026-10-14T16:30:00Z')
export const notesZone = { locale: 'en-GB', timeZone: 'UTC' }
export const viewer = 'person/example'
export const target = 'agent/example/reviewer'

const decode = decodeUnknownSync(SubjectNote)
let sequence = 0
const wire = (fields: { readonly author: string; readonly text: string; readonly audience: string; readonly created_at: string; readonly session?: string; readonly expires_at?: string; readonly target?: string }) => {
  sequence += 1
  const suffix = String(sequence).padStart(12, '0')
  return decode({
    id: `note/${fields.author}/019a0000-0000-7000-8000-${suffix}`,
    target: fields.target ?? target,
    author: fields.author,
    text: fields.text,
    audience: fields.audience,
    session: fields.session ?? null,
    created_at: fields.created_at,
    updated_at: fields.created_at,
    expires_at: fields.expires_at ?? null,
    revision: `claim/note-example-${suffix}`,
  })
}

export const retireNote = wire({ author: viewer, audience: 'self', text: 'this agent session should be retired soon', session: 'incarnation/example', created_at: '2026-10-14T16:20:00Z' })
export const diffNote = wire({ author: viewer, audience: 'agent', text: 'Prefer the smaller diff; the reviewer wants one pull request.', created_at: '2026-10-14T15:00:00Z', expires_at: '2026-10-15T09:00:00Z' })
export const agentNote = wire({ author: target, audience: 'agent', text: 'Waiting on the schema regeneration before the next push.', created_at: '2026-10-14T12:00:00Z' })
/** A future audience: the kit must show its neutral label, never this raw value. */
export const futureAudience = 'mission-crew'
export const futureNote = wire({ author: 'agent/example/planner', audience: futureAudience, text: 'Crew-wide context for the next run.', created_at: '2026-10-14T09:00:00Z' })
export const longNote = wire({
  author: viewer, audience: 'agent', created_at: '2026-10-13T18:00:00Z',
  text: `Context for the handoff:\n${'keep-the-selected-resource-visible-and-the-focus-where-the-reader-left-it-'.repeat(6)}end.\nSecond line stays on its own line.`,
})

/** Newest first: retire, diff, agent, future, long. Passed shuffled to prove the kit orders them. */
export const allNotes = [agentNote, longNote, retireNote, futureNote, diffNote] as const
export const newestFirstIds = [retireNote, diffNote, agentNote, futureNote, longNote].map(note => note.id)


/** A note as the host would publish it after a successful create, stamped at the fixed story clock. */
export const publishedNote = (text: string, audience: string, onTarget: string = target) =>
  wire({ author: viewer, audience, text, target: onTarget, created_at: new Date(notesNow).toISOString() })
export const plannerTarget = 'agent/example/planner'
export const plannerNote = wire({ author: viewer, audience: 'self', target: plannerTarget, text: 'Check the run budget before the next mission step.', created_at: '2026-10-14T14:10:00Z' })