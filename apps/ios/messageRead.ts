import { notApplied, retryTransient, type Attention, type FractalClient } from '../../clients/typescript/fractal-client';

/** Shared by the Home action and phone control; the same acted bookkeeping, fresh fence and retries. */
export const markMessageRead = async (client: Pick<FractalClient, 'messageRead' | 'capabilities'>, item: Attention, acted: Set<string>, actionId: () => string): Promise<void> => {
  acted.add(item.id);
  await retryTransient(8, async () => {
    const id = actionId();
    const capability = await client.capabilities();
    await client.messageRead({ id, idempotency_key: id, fence: { snapshot_id: capability.snapshot.id, subject_revisions: { [item.id]: item.revision } }, parameters: { target_id: item.source_id } });
  }, notApplied);
};
