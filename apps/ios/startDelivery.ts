import { appliedAnswer, ClientError, sendBoundAction, type BoundAction, type DeliveryOutcome, type DeliveryPorts } from '../../clients/typescript/fractal-client';

/** A once-signed Start needs explicit nonapplication proof before its recipient can change. */
export async function sendStart<T extends { fence: unknown }>(member: string, ports: DeliveryPorts<T> & { build(): Promise<T>; fence(): Promise<T['fence']> }): Promise<DeliveryOutcome<BoundAction<T>>> {
  let dispatched: T | undefined;
  const boundPorts = { ...ports, retry: false as const, send: (request: T) => {
    dispatched = request;
    return ports.send(request);
  } };
  const result = await sendBoundAction(member, boundPorts);
  if (dispatched && result.how === 'failed' && result.error instanceof ClientError && appliedAnswer(result.error) !== 'none') {
    return { how: 'unconfirmed', request: { member, request: dispatched, ports: boundPorts } };
  }
  return result;
}
