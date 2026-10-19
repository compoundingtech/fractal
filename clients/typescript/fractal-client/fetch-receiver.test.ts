import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { St3Client } from './Client.generated';
import type { TerminalSocket } from './Client.generated';

test('uses the global receiver for the default browser fetch', async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async function (this: typeof globalThis, url, init) {
        if (this !== globalThis) throw new TypeError('Illegal invocation');
        assert.equal(url, 'https://example.test/v1/client/capabilities');
        assert.equal(init?.method, 'GET');
        return Response.json({
            api_version: 'st3.client.v0',
            request_id: 'request/receiver',
            snapshot: { id: 'snapshot/test', host_id: 'host/test', store_index: 1, projection_version: 'client-projection.v0', created_at: '2026-09-20T00:00:00Z' },
            value: {},
        });
    };
    try {
        const client = new St3Client({ baseUrl: 'https://example.test/' });
        const result = await client.capabilities();
        assert.equal(result.request_id, 'request/receiver');
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('HTTP and stream receivers accept both API names and reject unknown or conflicting names', async () => {
    for (const api_version of ['st3.client.v0', 'fractal.client.v0', 'fractal.client.v1',
        'smalltalk.client.v0', ['st3.client.v0', 'fractal.client.v0']]) {
        const accepted = api_version === 'st3.client.v0' || api_version === 'fractal.client.v0';
        const snapshot = { id: 'snapshot/test' };
        const client = new St3Client({ baseUrl: 'https://example.test',
            fetchImpl: async () => Response.json({ api_version, snapshot, value: {} }) });
        if (accepted) assert.equal((await client.capabilities()).api_version, api_version);
        else await assert.rejects(client.capabilities(), /Unexpected st client API response/);
        for (const surface of ['terminal', 'conversation'] as const) {
            const received: unknown[] = [];
            const ended: (Error | undefined)[] = [];
            const socket: TerminalSocket = {
                onmessage: null, onclose: null, onerror: null, close: () => {},
            };
            const factory = () => socket;
            if (surface === 'terminal') {
                await client.terminalStream('terminal/test', { socket: factory, streamCapability: 'secret',
                    incarnation: 'incarnation/test', onScreen: value => received.push(value), onEnd: error => ended.push(error) });
            } else {
                await client.conversationStream('agent/test', { socket: factory,
                    onChange: value => received.push(value), onEnd: error => ended.push(error) });
            }
            socket.onmessage?.({ data: JSON.stringify({ api_version, snapshot,
                value: { kind: surface === 'terminal' ? 'terminal-screen' : 'conversation-changes' } }) });
            assert.equal(received.length, accepted ? 1 : 0, surface);
            assert.equal(ended.length, accepted ? 0 : 1, surface);
        }
    }
});
