import type { NoteAudience } from '@smalltalk/st3-client/schema'

/** Audiences a composer can write; the wire enum stays open for future audiences. */
export type NoteAudienceChoice = 'self' | 'agent'

export interface AudiencePresentation {
  /** Presentation family: `other` covers every audience this kit does not know yet. */
  readonly kind: NoteAudienceChoice | 'other'
  /** Visible label; never the raw wire value. Spells out the real sharing scope. */
  readonly label: string
  /** Compact visible label for dense chrome (badge preview chip); `label` stays the accessible name. */
  readonly shortLabel: string
  /** Accessible sentence for readers that need the consequence spelled out. Never claims delivery. */
  readonly description: string
}

const self: AudiencePresentation = { kind: 'self', label: 'Only me', shortLabel: 'Only me', description: 'Only you can read this note.' }
const agent: AudiencePresentation = {
  kind: 'agent', label: 'Shared with the agent (people can see it too)', shortLabel: 'Shared',
  description: 'The agent can read this note, and so can other people who can see this target. Mission recipients can change.',
}
const other: AudiencePresentation = { kind: 'other', label: 'Other audience', shortLabel: 'Other audience', description: 'This note has an audience this client does not describe yet.' }

/** True when any note on the target is shared with the agent, whichever note is newest. */
export const includesAgentShared = (notes: readonly { readonly audience: NoteAudience }[]): boolean => notes.some(note => note.audience === 'agent')

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
