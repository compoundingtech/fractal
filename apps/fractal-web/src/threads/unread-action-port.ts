import type { FractalClient } from '@fractal/client'
import * as Native from '@fractal/client/schema/read'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import type * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import type { Feed } from '../data/source.ts'
import type { ThreadObservation } from './threadSource.ts'

/** Canonical generated shapes; mark IDs remain opaque stable signed operation IDs. */
export type ThreadUnreadMark = Native.ThreadUnreadMark
export type ThreadUnreadReminder = Native.ThreadUnreadReminder

/** An unavailable read is not evidence that a thread has no reminder. */
export type UnreadReminderRead =
  | { readonly _tag: 'Known'; readonly reminder: ThreadUnreadReminder | undefined }
  | { readonly _tag: 'Unavailable'; readonly detail: string }

export type UnreadActionResult =
  | { readonly _tag: 'Sent' }
  | { readonly _tag: 'Unavailable'; readonly detail: string }
  | { readonly _tag: 'Refused'; readonly detail: string }

/** A pending signed intent has no visible message body and is not an active reminder. */
export type UnreadSeekResult =
  | { readonly _tag: 'Found'; readonly messageId: string }
  | { readonly _tag: 'Pending'; readonly detail: string }
  | { readonly _tag: 'Unavailable'; readonly detail: string }

export interface UnreadActionPort {
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

type UnreadAccess = {
  readonly read: boolean
  readonly actions: ReadonlySet<string>
  readonly actor?: string
}

type UnreadEdit =
  | { readonly kind: 'thread.mark-unread'; readonly anchor_message_id: string; readonly observed_marks: readonly string[] }
  | { readonly kind: 'thread.clear-unread'; readonly observed_marks: readonly string[] }

export type GatewayUnreadActionsOptions = {
  readonly client: FractalClient
  readonly registry: AtomRegistry.AtomRegistry
  readonly access: Atom.Atom<UnreadAccess>
  readonly detail: (threadId: string) => Atom.Atom<Feed<ThreadObservation>>
  /** Uses the source's existing bound delivery, grant, fence and receipt flow. */
  readonly send: (thread: ThreadObservation, edit: UnreadEdit) => Promise<UnreadActionResult>
  /** Only a current anchor with its real message body may replace the visible timeline. */
  readonly onFound: (threadId: string, page: Native.ThreadTimelinePage) => void
}

const unavailable = (detail: string): { readonly _tag: 'Unavailable'; readonly detail: string } => ({ _tag: 'Unavailable', detail })
const canEdit = (access: UnreadAccess): boolean => access.read
  && access.actions.has('thread.mark-unread') && access.actions.has('thread.clear-unread')

/** Reminder edits never acknowledge natural thread.read history or derive mark IDs from action receipts. */
export const gatewayUnreadActions = ({ client, registry, access, detail, send, onFound }: GatewayUnreadActionsOptions): UnreadActionPort => {
  const reminder = Atom.family((threadId: string) => Atom.make<UnreadReminderRead>(get => {
    const grant = get(access)
    const observed = get(detail(threadId))
    if (!grant.read || observed._tag !== 'Observed' || observed.freshness !== 'live') {
      return unavailable('Reconnect and load this thread before using unread reminders.')
    }
    return { _tag: 'Known', reminder: Option.getOrUndefined(observed.value.summary.unread_reminder) }
  }))
  const edit = (threadId: string, intent: UnreadEdit): Promise<UnreadActionResult> => {
    if (!canEdit(registry.get(access))) return Promise.resolve(unavailable('This connection cannot change unread reminders.'))
    const observed = registry.get(detail(threadId))
    if (observed._tag !== 'Observed' || observed.freshness !== 'live') {
      return Promise.resolve(unavailable('Reconnect and load this thread before changing its unread reminder.'))
    }
    return send(observed.value, intent)
  }
  const seeks = new Map<string, object>()
  return {
    get canMark() { return canEdit(registry.get(access)) },
    reminder,
    mark: ({ threadId, anchor_message_id, observed_marks }) => edit(threadId, {
      kind: 'thread.mark-unread', anchor_message_id, observed_marks: [...observed_marks],
    }),
    clear: ({ threadId, observed_marks }) => edit(threadId, { kind: 'thread.clear-unread', observed_marks: [...observed_marks] }),
    seek: async ({ threadId, anchor_message_id }) => {
      const grant = registry.get(access)
      if (!grant.read) return unavailable('This connection cannot read this thread.')
      const seek = {}
      seeks.set(threadId, seek)
      try {
        const { value } = await client.threadTimeline(threadId, { around: anchor_message_id, limit: 40 })
        const page = Native.decodeUnknownSync(Native.ThreadTimelinePage)(value)
        const current = registry.get(access)
        if (seeks.get(threadId) !== seek || !current.read || current.actor !== grant.actor) {
          return unavailable('The thread connection changed. Open the reminder again.')
        }
        if (page.anchor?.message_id !== anchor_message_id) return unavailable('Fractal could not locate this reminder’s message.')
        if (page.anchor.state === 'pending') return { _tag: 'Pending', detail: 'This reminder’s message has not arrived yet.' }
        if (page.anchor.state !== 'current') return unavailable('This reminder’s message is no longer available.')
        if (!page.items.some(item => item.kind === 'message' && item.id === anchor_message_id)) {
          return unavailable('Fractal did not return this reminder’s message.')
        }
        onFound(threadId, page)
        return { _tag: 'Found', messageId: anchor_message_id }
      } catch {
        return unavailable('Fractal could not load this reminder’s message. Try again once it reconnects.')
      } finally {
        if (seeks.get(threadId) === seek) seeks.delete(threadId)
      }
    },
  }
}
