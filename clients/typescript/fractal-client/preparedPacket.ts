import { API_VERSION } from './Models.generated.ts';
import type { ActionOf } from './Models.generated.ts';

export type PreparedMessageSendPacket = ActionOf<'message.send'>;
export type PreparedThreadStartPacket = ActionOf<'thread.start'>;
export type PreparedThreadReplyPacket = ActionOf<'thread.reply'>;
export type PreparedPacket = PreparedMessageSendPacket | PreparedThreadStartPacket | PreparedThreadReplyPacket;

/**
 * Complete action POST bodies, ready for persistence before dispatch. Callers own
 * action identity, fence selection and any signing; unsigned inputs stay unsigned.
 * No transport, credential or send closure is captured. Persist the returned body
 * and submit it unchanged through submitAction, subject to outbox delivery policy.
 * Treat supplied parameters and fences as immutable after preparation.
 */
export const prepareMessageSendPacket = (
    input: Omit<PreparedMessageSendPacket, 'api_version' | 'type'>,
): PreparedMessageSendPacket => ({
    ...input,
    api_version: API_VERSION,
    type: 'message.send',
});

/** Retains the full prepared Start, including its target, origin and optional signature. */
export const prepareThreadStartPacket = (
    input: Omit<PreparedThreadStartPacket, 'api_version' | 'type'>,
): PreparedThreadStartPacket => ({
    ...input,
    api_version: API_VERSION,
    type: 'thread.start',
});

/** Retains the full prepared reply, including its quote and optional signature. */
export const prepareThreadReplyPacket = (
    input: Omit<PreparedThreadReplyPacket, 'api_version' | 'type'>,
): PreparedThreadReplyPacket => ({
    ...input,
    api_version: API_VERSION,
    type: 'thread.reply',
});
