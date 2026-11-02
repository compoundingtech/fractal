import { notApplied, retryTransient, type Attention, type FractalClient } from '../../clients/typescript/fractal-client';
import type { PhoneCheck } from './phoneCommands';

/** Phone control reads the named message through the existing message.read action, not Home attention. */
export const readPhoneMessage = async (client: Pick<FractalClient, 'messageRead' | 'capabilities'>, messageId: string, actionId: () => string, check: PhoneCheck, deviceCommand: string): Promise<void> => {
  await retryTransient(8, async () => {
    check();
    const id = actionId();
    const capability = await client.capabilities();
    check();
    await client.messageRead({ id, idempotency_key: id, fence: { snapshot_id: capability.snapshot.id, subject_revisions: {} }, parameters: { target_id: messageId }, device_command: deviceCommand });
    check();
  }, notApplied);
};

/** Home's attention action retains its acted bookkeeping, fresh fence and retries. */
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
