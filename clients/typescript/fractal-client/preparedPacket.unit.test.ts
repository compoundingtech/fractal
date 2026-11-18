import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import { threadMessageSubject } from '../fractal-views/deviceSigning.ts';
import { API_VERSION } from './Models.generated.ts';
import type { DeviceSignature, Fence } from './Models.generated.ts';
import {
    expectedPreparedPacketIdentities,
    prepareMessageSendPacket,
    prepareThreadReplyPacket,
    prepareThreadStartPacket,
} from '@fractal/client/prepared-packet';
import type {
    PreparedMessageSendPacket,
    PreparedPacket,
    PreparedThreadReplyPacket,
    PreparedThreadStartPacket,
} from '@fractal/client/prepared-packet';

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

const sha256 = async (bytes: Uint8Array): Promise<Uint8Array> =>
    createHash('sha256').update(bytes).digest();

describe('expected prepared packet identities', () => {
    it('derives chat identity from only the original idempotency key', async () => {
        const packet = prepareMessageSendPacket(message);
        const identities = await expectedPreparedPacketIdentities(packet, 'person/sender', sha256);

        expect(identities).toStrictEqual(['message/4042fba54f182e8a']);
        expect(JSON.parse(JSON.stringify(identities))).toStrictEqual(identities);
        expect(await expectedPreparedPacketIdentities(packet, 'person/another', sha256)).toStrictEqual(identities);
        expect(await expectedPreparedPacketIdentities(
            { ...packet, idempotency_key: 'key/another' }, 'person/sender', sha256,
        )).not.toStrictEqual(identities);
    });

    const threadVectors = [
        {
            packet: prepareThreadStartPacket(start),
            sender: 'person/sender',
            expected: 'message/84d0f800721ff4f1',
        },
        {
            packet: prepareThreadReplyPacket(reply),
            sender: 'person/sender',
            expected: 'message/cc7069ea23151192',
        },
        {
            packet: prepareThreadReplyPacket({
                ...reply,
                idempotency_key: 'key/clé🔑',
                parameters: { ...reply.parameters, target_id: 'thread/café/🧵' },
            }),
            sender: 'person/发送者🙂',
            expected: 'message/526ce50008bb644a',
        },
    ];

    for (const { packet, sender, expected } of threadVectors) {
        it(`matches the ${packet.type} signing algorithm and vector ${expected}`, async () => {
            const fields = {
                content: packet.parameters.content,
                from: sender,
                thread_id: packet.parameters.target_id,
                to: 'agent/recipient',
            };
            const signingSubject = await threadMessageSubject(
                packet.type === 'thread.start'
                    ? {
                        ...fields,
                        title: packet.parameters.title,
                        thread_origin_source: packet.parameters.origin.source_id,
                        thread_origin_episode: packet.parameters.origin.episode,
                    }
                    : fields,
                packet.idempotency_key,
                sha256,
            );
            const identities = await expectedPreparedPacketIdentities(packet, sender, sha256);

            expect(identities).toStrictEqual([expected]);
            expect(identities).toStrictEqual([signingSubject]);
            expect(identities).not.toContain(packet.parameters.target_id);
            expect(JSON.parse(JSON.stringify(identities))).toStrictEqual(identities);
        });
    }

    it('binds thread identities to sender, key, thread and action', async () => {
        const packet = prepareThreadReplyPacket(reply);
        const original = await expectedPreparedPacketIdentities(packet, 'person/sender', sha256);
        const variants = [
            { packet, sender: 'person/another' },
            { packet: { ...packet, idempotency_key: 'key/another' }, sender: 'person/sender' },
            {
                packet: { ...packet, parameters: { ...packet.parameters, target_id: 'thread/another' } },
                sender: 'person/sender',
            },
            {
                packet: prepareThreadStartPacket({
                    ...start,
                    idempotency_key: packet.idempotency_key,
                    parameters: { ...start.parameters, target_id: packet.parameters.target_id },
                }),
                sender: 'person/sender',
            },
        ];
        for (const variant of variants) {
            expect(await expectedPreparedPacketIdentities(variant.packet, variant.sender, sha256))
                .not.toStrictEqual(original);
        }
    });

    it('length-prefixes UTF-8 parts rather than concatenating them', async () => {
        const packet = prepareThreadReplyPacket({
            ...reply,
            parameters: { ...reply.parameters, target_id: 'bc' },
        });
        const other = { ...packet, parameters: { ...packet.parameters, target_id: 'c' } };

        expect(await expectedPreparedPacketIdentities(packet, 'a', sha256))
            .not.toStrictEqual(await expectedPreparedPacketIdentities(other, 'ab', sha256));
    });

    const packets: readonly PreparedPacket[] = [
        prepareMessageSendPacket(message),
        prepareThreadStartPacket(start),
        prepareThreadReplyPacket(reply),
    ];
    for (const packet of packets) {
        it(`${packet.type} derives the same identity signed or unsigned without modifying either body`, async () => {
            // A supplied signature is not the trusted source of the original sender.
            const signatureFields = { signature: { ...signature, signer: 'person/not-original' } };
            const signed = packet.type === 'message.send'
                ? { ...packet, parameters: { ...packet.parameters, ...signatureFields } }
                : packet.type === 'thread.start'
                    ? { ...packet, parameters: { ...packet.parameters, ...signatureFields } }
                    : { ...packet, parameters: { ...packet.parameters, ...signatureFields } };
            const originalBody = JSON.stringify(packet);
            const signedBody = JSON.stringify(signed);
            const identities = await expectedPreparedPacketIdentities(packet, 'person/sender', sha256);

            expect(await expectedPreparedPacketIdentities(signed, 'person/sender', sha256)).toStrictEqual(identities);
            expect(JSON.stringify(packet)).toBe(originalBody);
            expect(JSON.stringify(signed)).toBe(signedBody);
            expect(Object.hasOwn(packet.parameters, 'signature')).toBe(false);
        });
    }
});
