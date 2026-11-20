import { visitControl, type UnreadEdit, type UnreadSummary, type VisitEffect, type VisitEvent, type VisitState, type VisitTransition } from './unreadVisit.ts';
import type { ThreadEditResult } from './threadEdits.ts';
import type { UnreadSeek } from './unreadClient.ts';

/** Domain ports, NOT proposed client-v0 wire parameters or action-result shapes. */
export interface UnreadActions {
  read(messageIds: readonly string[]): Promise<UnreadActionResult>;
  edit(edit: UnreadEdit): Promise<UnreadActionResult>;
  seek(anchorMessageId: string): Promise<UnreadSeek>;
}
export type UnreadActionResult = ThreadEditResult;
export type UnreadAPI = { readonly state: 'pending-api' } | { readonly state: 'ready'; readonly actions: UnreadActions };
/** Unbound ports are retained for clients without the thread capability. Production
 * clients bind generated calls in unreadClient, with the store's bound retries.
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
