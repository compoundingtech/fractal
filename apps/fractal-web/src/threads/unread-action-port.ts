import type { FractalClient } from '@fractal/client'
import * as Atom from 'effect/reactivity/Atom'

/** An opaque, stable signed mark operation ID and the message it names; IDs come only from the reminder read. */
export type ThreadUnreadMark = {
  readonly mark_id: string
  readonly anchor_message_id: string
}

export type ThreadUnreadReminder = {
  readonly marks: readonly ThreadUnreadMark[]
  readonly anchor_message_id: string
}

/** An unavailable read is not evidence that a thread has no reminder. */
export type UnreadReminderRead =
  | { readonly _tag: 'Known'; readonly reminder: ThreadUnreadReminder | undefined }
  | { readonly _tag: 'Unavailable'; readonly detail: string }

export type UnreadActionResult =
  | { readonly _tag: 'Sent' }
  | { readonly _tag: 'Unavailable'; readonly detail: string }
  | { readonly _tag: 'Refused'; readonly detail: string }

/** Seeking locates an authoritative message; it never supplies a fabricated message body. */
export type UnreadSeekResult =
  | { readonly _tag: 'Found'; readonly messageId: string }
  | { readonly _tag: 'Unavailable'; readonly detail: string }

export interface UnreadActionPort {
  /** Requires the reminder API and its own grant, independently of natural thread.read receipts. */
  readonly canMark: boolean
  readonly reminder: (threadId: string) => Atom.Atom<UnreadReminderRead>
  readonly mark: (request: {
    readonly threadId: string
    readonly anchor_message_id: string
    readonly observed_marks: readonly string[]
  }) => Promise<UnreadActionResult>
  readonly clear: (request: {
    readonly threadId: string
    readonly observed_marks: readonly string[]
  }) => Promise<UnreadActionResult>
  readonly seek: (request: {
    readonly threadId: string
    readonly anchor_message_id: string
  }) => Promise<UnreadSeekResult>
}

const unsupported = {
  _tag: 'Unavailable',
  detail: 'This connection does not support unread reminders.',
} as const

/**
 * The generated gateway contract has no unread reminder reads, actions or anchor seek yet.
 * Do not infer marks from thread.read, mint operation IDs, or send guessed network requests.
 * Keep the unsupported read explicit until those authoritative helpers are available.
 */
export const gatewayUnreadActions = (_client: FractalClient): UnreadActionPort => ({
  canMark: false,
  reminder: Atom.family((_threadId: string) => Atom.make<UnreadReminderRead>(unsupported)),
  mark: async () => unsupported,
  clear: async () => unsupported,
  seek: async () => unsupported,
})
