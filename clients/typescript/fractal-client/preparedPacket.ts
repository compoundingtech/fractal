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

/**
 * Derives the exact message identities to persist with an outbox record, without
 * signing or dispatching the packet. Supply a platform SHA-256 implementation.
 * `sender` must be the captured, trusted original actor, including for unsigned
 * thread packets: neither a signature nor the currently active actor supplies it.
 * A thread ID is not delivery evidence; only its derived message identity is.
 */
export const expectedPreparedPacketIdentities = async (
    packet: PreparedPacket,
    sender: string,
    sha256: (bytes: Uint8Array) => Promise<Uint8Array>,
): Promise<readonly string[]> => {
    const utf8 = new TextEncoder();
    const hex = (bytes: Uint8Array): string =>
        Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
    let key = packet.idempotency_key;
    if (packet.type !== 'message.send') {
        const action = packet.type === 'thread.start' ? 'start' : 'reply';
        const parts = ['fractal.thread-message.v1', sender, packet.parameters.target_id, action, key]
            .map(part => utf8.encode(part));
        const input = new Uint8Array(parts.reduce((size, part) => size + 8 + part.length, 0));
        const prefix = new DataView(input.buffer);
        let offset = 0;
        for (const part of parts) {
            prefix.setBigUint64(offset, BigInt(part.length), false);
            input.set(part, offset + 8);
            offset += 8 + part.length;
        }
        key = `thread-message:${hex(await sha256(input))}`;
    }
    return [`message/${hex(await sha256(utf8.encode(key))).slice(0, 16)}`];
};
