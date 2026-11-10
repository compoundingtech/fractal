import type { Arrangement } from '../../clients/typescript/fractal-client';
import type { MembershipComplete } from './sidebarMemberships';
import { buildTree, type ProjectionError, type Tree } from './sidebarPlanning';

export type ProjectionInput = {
  arrangement: Arrangement;
  complete: MembershipComplete;
  // Covers raw folder/register changes not represented by arrangement revision;
  // resolved parents are independently checked even when this revision is unchanged.
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
  private tree?: Tree;
  private folderCount = 0;
  clear(): void { this.key = undefined; this.tree = undefined; this.folderCount = 0; }
  get(input: ProjectionInput): Tree | { error: ProjectionError } {
    const { arrangement, complete, modelRevision, catalogRevision, presentationRevision } = input;
    const { scope, frontier } = complete;
    if (arrangement.id !== scope.container || arrangement.owner !== scope.owner) {
      this.clear(); return { error: { _tag: 'InvalidResolution' } };
    }
    if (arrangement.deleted || !arrangement.resolved) {
      this.clear(); return buildTree(arrangement, complete.members);
    }
    let folderCount = 0, parentsChanged = false;
    // Admission bounds this scan to 1,024 live folders; never inspect members on a hit.
    for (const id in arrangement.body.folders) {
      if (arrangement.body.folders[id].tombstone != null) continue;
      folderCount++;
      if (!Object.hasOwn(arrangement.resolved.parents, id)) {
        this.clear(); return buildTree(arrangement, complete.members);
      }
      const parent = arrangement.resolved.parents[id], previous = this.tree?.nodes[`folder:${id}`]?.shownUnder;
      if (!previous || (parent === null ? previous._tag !== 'Root' : previous._tag !== 'Folder' || previous.id !== parent)) parentsChanged = true;
    }
    const key = JSON.stringify([scope.owner, scope.container, scope.endpoint, scope.host, scope.projection,
      arrangement.revision, arrangement.deleted ?? false, arrangement.body.version, modelRevision,
      frontier.epoch, frontier.changed_index, frontier.live_count, catalogRevision, presentationRevision]);
    if (key !== this.key || parentsChanged || folderCount !== this.folderCount) {
      this.clear();
      const tree = buildTree(arrangement, complete.members);
      if ('error' in tree) { this.clear(); return tree; }
      this.tree = tree; this.key = key; this.folderCount = folderCount;
    }
    return this.tree!;
  }
}
