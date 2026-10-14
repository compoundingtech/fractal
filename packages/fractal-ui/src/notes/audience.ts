import type { NoteAudience } from '@smalltalk/st3-client/schema'

/** Audiences a composer can write; the wire enum stays open for future audiences. */
export type NoteAudienceChoice = 'self' | 'agent'

export interface AudiencePresentation {
  /** Presentation family: `other` covers every audience this kit does not know yet. */
  readonly kind: NoteAudienceChoice | 'other'
  /** Visible label; never the raw wire value. */
  readonly label: string
  /** Accessible sentence for readers that need the consequence spelled out. */
  readonly description: string
}

const self: AudiencePresentation = { kind: 'self', label: 'Only me', description: 'Only you can read this note.' }
const agent: AudiencePresentation = { kind: 'agent', label: 'Shared with agent', description: 'The agent reads this note at its next turn.' }
const other: AudiencePresentation = { kind: 'other', label: 'Other audience', description: 'This note has an audience this client does not describe yet.' }

/** The single audience label mapping. Unrecognized audiences render neutrally, never as their raw value. */
export const audiencePresentation = (audience: NoteAudience | NoteAudienceChoice): AudiencePresentation => {
  if (typeof audience !== 'string') return other
  switch (audience) {
    case 'self': return self
    case 'agent': return agent
    default: {
      const exhaustive: never = audience
      return exhaustive
    }
  }
}
