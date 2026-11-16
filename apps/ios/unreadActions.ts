import { visitControl, type Anchor, type UnreadEdit, type UnreadSummary, type VisitEffect, type VisitEvent, type VisitState, type VisitTransition } from './unreadVisit.ts';
import type { ThreadEditResult } from './threadEdits.ts';

/** Domain ports, NOT proposed client-v0 wire parameters or action-result shapes. */
export interface UnreadActions {
  read(messageIds: readonly string[]): Promise<UnreadActionResult>;
  edit(edit: UnreadEdit): Promise<UnreadActionResult>;
  seek(anchorMessageId: string): Promise<Anchor>;
}
export type UnreadActionResult = ThreadEditResult;
export type UnreadAPI = { readonly state: 'pending-api' } | { readonly state: 'ready'; readonly actions: UnreadActions };
/** TODO(client-v0): bind generated mark/clear/around calls and their bound retries here.
 * Never reuse thread.read as a clear, mint mark IDs, or use action-result.operation_id.
 * The adapter must retain the original signed request for unknown-outcome retries.
 */
export const pendingUnreadAPI: UnreadAPI = { state: 'pending-api' };

/** Check grants at execution too: a screen's captured button can outlive revocation. */
export const applyVisitEffect = async (
  effect: VisitEffect, api: UnreadAPI, grants: { canRead: boolean; canEdit: boolean },
): Promise<UnreadActionResult> => {
  if (effect.kind === 'read' ? !grants.canRead : !grants.canEdit)
    return { how: 'failed', error: 'This connection is read-only.' };
  if (api.state === 'pending-api') return { how: 'failed', error: 'Unread reminder actions require the pending client API.' };
  return effect.kind === 'read' ? api.actions.read(effect.messageIds) : api.actions.edit(effect.edit);
};

/** Screen-local lifecycle owner. Transitions are pure; UI supplies measured evidence.
 * The summary is the ONLY reminder/ID input, including after confirmed actions.
 */
export class UnreadVisitController {
  private current: VisitState;
  constructor(state: VisitState) { this.current = state; }
  get state(): VisitState { return this.current; }
  send(event: VisitEvent): VisitTransition {
    const transition = visitControl(this.current, event);
    this.current = transition.state;
    return transition;
  }
  observe(summary: UnreadSummary): void { this.send({ kind: 'summary', summary }); }
}
