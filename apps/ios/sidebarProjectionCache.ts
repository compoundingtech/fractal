import type { Arrangement } from '../../clients/typescript/fractal-client';
import type { MembershipComplete } from './sidebarMemberships';
import { buildTree, type ProjectionError, type Tree } from './sidebarPlanning';

export type ProjectionInput = {
  arrangement: Arrangement;
  complete: MembershipComplete;
  // Host revisions cover resolved/model changes and all catalog/presentation inputs.
  modelRevision: string;
  catalogRevision: string;
  presentationRevision: string;
};
/** Unwired, single-entry host-owned cache. Inputs must be complete and revisioned;
 * returned trees are borrowed and must not be mutated. Snapshot/store indexes are
 * deliberately excluded: unrelated daemon writes do not change this collection.
 * A host must clear on unavailable data and may retain stale trees separately. */
export class SidebarProjectionCache {
  private key?: string;
  private tree?: Tree | { error: ProjectionError };
  clear(): void { this.key = undefined; this.tree = undefined; }
  get(input: ProjectionInput): Tree | { error: ProjectionError } {
    const { arrangement, complete, modelRevision, catalogRevision, presentationRevision } = input;
    const { scope, frontier } = complete;
    if (arrangement.id !== scope.container || arrangement.owner !== scope.owner) {
      this.clear(); return { error: { _tag: 'InvalidResolution' } };
    }
    const key = JSON.stringify([scope.owner, scope.container, scope.endpoint, scope.host, scope.projection,
      arrangement.revision, arrangement.deleted ?? false, arrangement.body.version, modelRevision,
      frontier.epoch, frontier.changed_index, frontier.live_count, catalogRevision, presentationRevision]);
    if (key !== this.key) {
      this.tree = buildTree(arrangement, complete.members);
      this.key = key;
    }
    return this.tree!;
  }
}
