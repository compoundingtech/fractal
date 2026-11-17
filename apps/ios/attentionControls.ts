import type { Attention } from '../../clients/typescript/fractal-client';
/** A dismissal is the person's outcome, never an approval or a substitute for another kind's action. */
export function dismissalKind(item: Attention & { closedElsewhere?: boolean }): 'dismiss' | 'answer' | 'read' | 'message' | null {
  if (item.closedElsewhere || item.state !== 'open') return null;
  if (item.actions.includes('message.read')) return 'message';
  if (item.attention_kind !== 'person-step' || !item.requester_id) return null;
  if (item.update && item.actions.includes('work.done')) return 'read';
  if (!item.request || !item.actions.includes('work.done')) return null;
  return item.request.answers?.some(answer => answer.id === 'dismiss') ? 'answer' : 'dismiss';
}
