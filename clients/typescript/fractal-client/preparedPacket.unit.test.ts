import { describe, expect, it } from 'vitest';
import { API_VERSION } from './Models.generated.ts';
import type { DeviceSignature, Fence } from './Models.generated.ts';
import {
    prepareMessageSendPacket,
    prepareThreadReplyPacket,
    prepareThreadStartPacket,
} from './preparedPacket.ts';
import type {
    PreparedMessageSendPacket,
    PreparedThreadReplyPacket,
    PreparedThreadStartPacket,
} from './preparedPacket.ts';

const fence = {
    snapshot_id: 'snapshot/original',
    subject_revisions: { 'agent/recipient': 'revision/agent', 'thread/original': 'revision/thread' },
    runtime_desired_revision: 'revision/runtime',
    runtime_incarnation: 'incarnation/original',
} satisfies Fence;

const signature = {
    chain: ['certificate/device', 'certificate/owner'],
    format: 'fields-v1',
    key: 'key/device',
    nonce: 'nonce/original',
    signature: 'original-signature-bytes',
    signed_at_unix_ms: 1_790_000_000_123,
    signed_fields: ['subject', 'content', 'recipient', 'nonce'],
    signer: 'person/sender',
} satisfies DeviceSignature;

const message = {
    id: 'action/chat-original',
    idempotency_key: 'key/chat-original',
    fence,
    parameters: {
        to: 'agent/recipient',
        content: 'Prepared chat\nwith original Unicode: café',
        attachments: [{ blob: 'blob/original', media_type: 'image/png', name: 'original.png' }],
        in_reply_to: 'message/quoted',
        kind: 'wake',
        session_id: 'session/original',
        tags: ['tag/one', 'tag/two'],
        title: 'Original title',
    },
} satisfies Omit<PreparedMessageSendPacket, 'api_version' | 'type'>;

const start = {
    id: 'action/start-original',
    idempotency_key: 'key/start-original',
    fence,
    parameters: {
        agent_id: 'agent/recipient',
        content: 'Prepared first message',
        origin: { source_id: 'inbox/source', episode: 'revision/episode' },
        target_id: 'thread/new-original',
        title: 'Original thread title',
    },
} satisfies Omit<PreparedThreadStartPacket, 'api_version' | 'type'>;

const reply = {
    id: 'action/reply-original',
    idempotency_key: 'key/reply-original',
    fence,
    parameters: {
        content: 'Prepared reply',
        target_id: 'thread/original',
        thread_quote_id: 'message/thread-quoted',
    },
} satisfies Omit<PreparedThreadReplyPacket, 'api_version' | 'type'>;

// Each fixture includes every optional payload field other than signature.
// Exercise both authorized unsigned web bodies and already-signed native bodies.
for (const signed of [false, true]) {
    describe(signed ? 'signed prepared packets' : 'unsigned prepared packets', () => {
        const signatureFields = signed ? { signature } : {};
        const cases = [
            {
                type: 'message.send',
                input: { ...message, parameters: { ...message.parameters, ...signatureFields } },
                prepare: () => prepareMessageSendPacket({
                    ...message,
                    parameters: { ...message.parameters, ...signatureFields },
                }),
            },
            {
                type: 'thread.start',
                input: { ...start, parameters: { ...start.parameters, ...signatureFields } },
                prepare: () => prepareThreadStartPacket({
                    ...start,
                    parameters: { ...start.parameters, ...signatureFields },
                }),
            },
            {
                type: 'thread.reply',
                input: { ...reply, parameters: { ...reply.parameters, ...signatureFields } },
                prepare: () => prepareThreadReplyPacket({
                    ...reply,
                    parameters: { ...reply.parameters, ...signatureFields },
                }),
            },
        ];

        for (const { type, input, prepare } of cases) {
            it(`${type} roundtrips its complete original action body`, () => {
                const packet = prepare();
                const expected = { ...input, api_version: API_VERSION, type };
                const restored = JSON.parse(JSON.stringify(packet));

                expect(packet).toStrictEqual(expected);
                expect(restored).toStrictEqual(expected);
                expect(packet.parameters).toStrictEqual(input.parameters);
                expect(packet.fence).toBe(fence);
                if (signed) {
                    expect(packet.parameters.signature).toBe(signature);
                    expect(restored.parameters.signature).toStrictEqual(signature);
                } else {
                    expect(Object.hasOwn(packet.parameters, 'signature')).toBe(false);
                    expect(Object.hasOwn(restored.parameters, 'signature')).toBe(false);
                }
            });
        }
    });
}
