import { sendBoundAction, type BoundAction, type DeliveryOutcome, type DeliveryPorts } from '../../clients/typescript/fractal-client';

/** A once-signed Start needs explicit nonapplication proof before its recipient can change. */
export async function sendStart<T extends { fence: unknown }>(member: string, ports: DeliveryPorts<T> & { build(): Promise<T>; fence(): Promise<T['fence']> }): Promise<DeliveryOutcome<BoundAction<T>>> {
  return sendBoundAction(member, { ...ports, retry: false });
}
