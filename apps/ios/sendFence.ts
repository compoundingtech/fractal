// Fractal accepts a snapshot from this host at any nonfuture store position. Subject revisions
// remain exact: using an already received snapshot removes a capabilities round trip, never the
// revision check. A retry after a refusal reads a fresh snapshot on the original connection.
export type SendFence = { snapshot_id: string; subject_revisions: Record<string, string> };

/** The first try uses the held snapshot; later tries read fresh and keep the viewed revisions. */
export function sendFences(held: string | undefined, fresh: () => Promise<SendFence>, revisions: Record<string, string> = {}): () => Promise<SendFence> {
  let first = held || undefined;
  const subjectRevisions = { ...revisions };
  return async () => {
    if (first) {
      const snapshot_id = first;
      first = undefined;
      return { snapshot_id, subject_revisions: { ...subjectRevisions } };
    }
    return { ...(await fresh()), subject_revisions: { ...subjectRevisions } };
  };
}
