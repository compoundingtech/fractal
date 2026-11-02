import { notApplied, retryTransient, type Attention, type FractalClient } from '../../clients/typescript/fractal-client';
import type { PhoneCheck } from './phoneCommands';

export const readPhoneMessage = async (id: string, attention: Attention[], markRead: (item: Attention) => Promise<boolean>): Promise<boolean> => {
  const item = attention.find(row => row.id === id && row.actions.includes('message.read'));
  return item ? markRead(item) : false;
};

/** Shared by the Home action and phone control; the same acted bookkeeping, fresh fence and retries. */
export const markMessageRead = async (client: Pick<FractalClient, 'messageRead' | 'capabilities'>, item: Attention, acted: Set<string>, actionId: () => string, check?: PhoneCheck, deviceCommand?: string): Promise<void> => {
  await retryTransient(8, async () => {
    check?.();
    const id = actionId();
    const capability = await client.capabilities();
    check?.();
    acted.add(item.id);
    await client.messageRead({ id, idempotency_key: id, fence: { snapshot_id: capability.snapshot.id, subject_revisions: { [item.id]: item.revision } }, parameters: { target_id: item.source_id }, ...(deviceCommand === undefined ? {} : { device_command: deviceCommand }) });
    check?.();
  }, notApplied);
};
