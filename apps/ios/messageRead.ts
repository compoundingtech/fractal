import { notApplied, retryTransient, type Attention, type FractalClient } from '../../clients/typescript/fractal-client';

export const readPhoneMessage = async (id: string, attention: Attention[], markRead: (item: Attention) => Promise<boolean>): Promise<boolean> => {
  const item = attention.find(row => row.id === id && row.actions.includes('message.read'));
  return item ? markRead(item) : false;
};

/** Shared by the Home action and phone control; the same acted bookkeeping, fresh fence and retries. */
export const markMessageRead = async (client: Pick<FractalClient, 'messageRead' | 'capabilities'>, item: Attention, acted: Set<string>, actionId: () => string, allowed: () => boolean = () => true): Promise<void> => {
  acted.add(item.id);
  await retryTransient(8, async () => {
    if (!allowed()) throw new Error('Phone command is no longer active');
    const id = actionId();
    const capability = await client.capabilities();
    if (!allowed()) throw new Error('Phone command is no longer active');
    await client.messageRead({ id, idempotency_key: id, fence: { snapshot_id: capability.snapshot.id, subject_revisions: { [item.id]: item.revision } }, parameters: { target_id: item.source_id } });
  }, notApplied);
};
