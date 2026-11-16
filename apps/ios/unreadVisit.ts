import type { ThreadSummary, ThreadTimelineAnchor, ThreadUnreadMark, ThreadUnreadReminder } from '../../clients/typescript/fractal-client';
import { decodeUnknownSync, ThreadUnreadReminder as ReminderSchema } from '../../clients/typescript/fractal-client/Schema.generated.ts';
export type { ThreadUnreadMark, ThreadUnreadReminder };
export type UnreadSummary = Pick<ThreadSummary, 'unread' | 'unread_reminder'>;
export type Anchor = ThreadTimelineAnchor;

/** Seek metadata alone is never content, read evidence, or badge authority. */
export const anchorPresentation = (anchor: Anchor): { body: boolean; notice?: string } => {
  switch (anchor.state) {
    case 'current': return { body: true };
    case 'unavailable': return { body: false, notice: 'The return message is unavailable. Your reminder is kept.' };
    case 'pending': return { body: false, notice: 'The return message is not known yet.' };
  }
};
export const reminderFromSummary = (summary: UnreadSummary | undefined): ThreadUnreadReminder | undefined => {
  const reminder = summary?.unread_reminder;
  if (!reminder) return undefined;
  // Canonical decoding is mandatory even for summaries arriving over the feed.
  // Preserve the generated wire DTO rather than its decoded Option fields.
  try { decodeUnknownSync(ReminderSchema, 'strict')(reminder); }
  catch { return undefined; }
  return reminder.marks.length ? reminder : undefined;
};

export type UnreadEdit =
  | { readonly kind: 'mark'; readonly anchor_message: string; readonly observed_marks: readonly string[] }
  | { readonly kind: 'clear'; readonly observed_marks: readonly string[] };
export type VisitEffect =
  | { readonly kind: 'read'; readonly messageIds: readonly string[] }
  | { readonly kind: 'edit'; readonly edit: UnreadEdit };
type Visit = {
  readonly foreground: boolean;
  readonly visible: boolean;
  /** Only marks captured on entry can auto-clear. New/concurrent marks stay sticky. */
  readonly captured: readonly ThreadUnreadMark[];
  readonly viewed: readonly string[];
  readonly read: readonly string[];
  readonly clearing: readonly string[];
};
export type VisitState = {
  readonly canRead: boolean;
  readonly canEdit: boolean;
  readonly reminder?: ThreadUnreadReminder;
  readonly visit?: Visit;
};
export type VisitEvent =
  | { readonly kind: 'start'; readonly foreground: boolean; readonly visible: boolean }
  | { readonly kind: 'end' }
  | { readonly kind: 'foreground'; readonly value: boolean }
  | { readonly kind: 'visibility'; readonly value: boolean }
  | { readonly kind: 'capabilities'; readonly canRead: boolean; readonly canEdit: boolean }
  | { readonly kind: 'summary'; readonly summary: UnreadSummary }
  | { readonly kind: 'meaningful-content-visible'; readonly messageIds: readonly string[]; readonly readMessageIds?: readonly string[]; readonly deliberate: boolean }
  | { readonly kind: 'mark-unread'; readonly anchor: Anchor }
  | { readonly kind: 'mark-read' }
  // Action receipts intentionally carry NO operation ID. Only a summary supplies mark IDs.
  | { readonly kind: 'edit-finished' }
  | { readonly kind: 'read-failed'; readonly messageIds: readonly string[] }
  | { readonly kind: 'edit-failed'; readonly observed_marks: readonly string[] };
export type VisitTransition = { readonly state: VisitState; readonly effects: readonly VisitEffect[]; readonly refusal?: string };
export const unreadVisit = (canRead: boolean, canEdit: boolean): VisitState => ({ canRead, canEdit });
const observed = (state: VisitState): readonly string[] => state.reminder?.marks.map(mark => mark.mark_id) ?? [];
const active = (state: VisitState): boolean => !!state.visit?.foreground && state.visit.visible;

/** No clocks, identity generation, persistence, network, or natural-history rewinds. */
export const visitControl = (state: VisitState, event: VisitEvent): VisitTransition => {
  const quiet = (next = state): VisitTransition => ({ state: next, effects: [] });
  switch (event.kind) {
    case 'start':
      // Entry is the only capture point, including Unknown entry summaries.
      // A remount/refocus may restore visibility but must never recapture marks.
      return quiet(state.visit ? { ...state, visit: { ...state.visit,
        foreground: event.foreground, visible: event.visible,
      } } : { ...state, visit: {
        foreground: event.foreground, visible: event.visible,
        captured: state.reminder?.marks.slice() ?? [], viewed: [], read: [], clearing: [],
      } });
    case 'end': return quiet({ ...state, visit: undefined });
    case 'foreground': return quiet(state.visit ? { ...state, visit: { ...state.visit, foreground: event.value, viewed: [] } } : state);
    case 'visibility': return quiet(state.visit ? { ...state, visit: { ...state.visit, visible: event.value, viewed: [] } } : state);
    case 'capabilities': return quiet({ ...state, canRead: event.canRead, canEdit: event.canEdit });
    case 'summary': return quiet({ ...state, reminder: reminderFromSummary(event.summary) });
    case 'edit-finished': return quiet(); // Never turn a receipt into an optimistic reminder.
    case 'read-failed': return quiet(state.visit ? { ...state, visit: { ...state.visit,
      read: state.visit.read.filter(id => !event.messageIds.includes(id)),
    } } : state);
    case 'edit-failed': return quiet(state.visit ? { ...state, visit: { ...state.visit,
      clearing: state.visit.clearing.filter(id => !event.observed_marks.includes(id)),
    } } : state);
    case 'mark-unread': {
      if (!state.canEdit) return { ...quiet(), refusal: 'This connection cannot change unread reminders.' };
      if (!active(state)) return { ...quiet(), refusal: 'Open this thread in the foreground first.' };
      // A pending intent can only come from the signed API, never from local guessing.
      if (event.anchor.state !== 'current' || !state.visit?.viewed.includes(event.anchor.message_id))
        return { ...quiet(), refusal: 'View the return message before marking unread.' };
      return { state, effects: [{ kind: 'edit', edit: {
        kind: 'mark', anchor_message: event.anchor.message_id, observed_marks: observed(state),
      } }] };
    }
    case 'mark-read': {
      if (!state.canEdit) return { ...quiet(), refusal: 'This connection cannot change unread reminders.' };
      if (!active(state)) return { ...quiet(), refusal: 'Open this thread in the foreground first.' };
      const ids = observed(state);
      return { state, effects: ids.length ? [{ kind: 'edit', edit: { kind: 'clear', observed_marks: ids } }] : [] };
    }
    case 'meaningful-content-visible': {
      const visit = state.visit;
      if (!visit || !active(state)) return quiet();
      const ids = [...new Set(event.messageIds)];
      const reading = state.canRead ? ids.filter(id => (event.readMessageIds ?? ids).includes(id) && !visit.read.includes(id)) : [];
      const captured = new Set(visit.captured.filter(mark => ids.includes(mark.anchor_message_id)).map(mark => mark.mark_id));
      const clearing = state.canEdit && event.deliberate
        ? observed(state).filter(id => captured.has(id) && !visit.clearing.includes(id)) : [];
      return { state: { ...state, visit: { ...visit, viewed: ids,
        read: [...visit.read, ...reading], clearing: [...visit.clearing, ...clearing],
      } }, effects: [
        ...(reading.length ? [{ kind: 'read' as const, messageIds: reading }] : []),
        ...(clearing.length ? [{ kind: 'edit' as const, edit: { kind: 'clear' as const, observed_marks: clearing } }] : []),
      ] };
    }
  }
};
