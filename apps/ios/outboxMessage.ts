import type { OutboxIntent } from '../../clients/typescript/fractal-client/outbox.ts';

export type OutboxMessage =
  | { kind: 'chat'; to: string; sessionId?: string; tags?: readonly string[] }
  | { kind: 'start'; target: string; title: string; origin: { agentId: string; sourceId: string; episode: string } }
  | { kind: 'reply'; target: string; agentId: string; quoteId?: string };
export type OutboxMessageMetadata = {
  message: OutboxMessage;
  binding: { profileSlot: string; person: string; signingKey: string; sessionActor: string };
  acceptedAt: number;
};
export type OutboxMessageIntent = OutboxIntent & OutboxMessageMetadata;
export type OutboxAccepted = { how: 'accepted'; outboxId: string };
export type OutboxRecoveryRow = {
  id: string;
  message?: OutboxMessage;
  text?: string;
  images: number;
  member: string;
  state: 'pending' | 'unknown' | 'delivered' | 'not-applied' | 'corrupt' | 'legacy';
  status: string;
  canCheck: boolean;
};
export type LegacyStartRecovery = {
  id: string; message: Extract<OutboxMessage, { kind: 'start' }>; text: string; serializedDraft: string;
};

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const fields = (value: unknown, keys: readonly string[]): boolean =>
  object(value) && keys.every(key => typeof value[key] === 'string' && value[key] !== '');
const optionalString = (value: unknown): boolean => value === undefined || typeof value === 'string';
/** Disk/legacy boundary only. Missing context is recovery content, never a sendable intent. */
export const messageIntent = (intent: OutboxIntent): OutboxMessageIntent | undefined => {
  const value: unknown = intent;
  if (!object(value) || !object(value.message) || !object(value.binding)
    || typeof value.acceptedAt !== 'number' || !Number.isSafeInteger(value.acceptedAt) || value.acceptedAt < 0
    || !fields(value.binding, ['profileSlot', 'person', 'signingKey', 'sessionActor'])) return;
  const message = value.message;
  switch (message.kind) {
    case 'chat':
      if (typeof message.to !== 'string' || !message.to || !optionalString(message.sessionId)
        || (message.tags !== undefined && (!Array.isArray(message.tags) || !message.tags.every(tag => typeof tag === 'string')))) return;
      break;
    case 'start':
      if (typeof message.target !== 'string' || !message.target || typeof message.title !== 'string' || !object(message.origin)
        || !fields(message.origin, ['agentId', 'sourceId', 'episode'])) return;
      break;
    case 'reply':
      if (typeof message.target !== 'string' || !message.target || typeof message.agentId !== 'string' || !message.agentId || !optionalString(message.quoteId)) return;
      break;
    default: return;
  }
  // All required fields of this extension were checked above; core decodes text/attachments.
  return intent as OutboxMessageIntent;
};
