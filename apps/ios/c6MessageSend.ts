import type { FractalClient } from '../../clients/typescript/fractal-client';
import { checkC6Aborted, type C6Send } from './c6Recorder';
import { sendBoundMessage, type BoundMessage, type SendOutcome } from './messageSend';

type Request = Parameters<FractalClient['messageSend']>[0];

/** The store's signed-send hooks, retaining the original transport and signing ports. */
export const sendSignedMessage = (member: string, ports: {
  client: FractalClient;
  c6Send?: C6Send;
  actionId: () => string;
  parameters: (id: string) => Promise<Request['parameters']>;
  fences: () => Promise<Request['fence']>;
  fence: () => Promise<Request['fence']>;
  originalAvailable: () => void;
  wait: (ms: number) => Promise<void>;
}): Promise<SendOutcome<BoundMessage<Request>>> => sendBoundMessage(member, {
  build: async () => {
    checkC6Aborted(ports.c6Send?.signal);
    const id = ports.actionId();
    ports.c6Send?.request(id);
    const parameters = await ports.parameters(id);
    return { id, idempotency_key: id, fence: await ports.fences(), parameters };
  },
  fence: async () => { checkC6Aborted(ports.c6Send?.signal); ports.originalAvailable(); return ports.fence(); },
  send: async request => {
    ports.originalAvailable(); checkC6Aborted(ports.c6Send?.signal);
    const result = await ports.client.messageSend(request, { signal: ports.c6Send?.signal });
    ports.c6Send?.ack();
    return result;
  },
  wait: ports.wait,
});
