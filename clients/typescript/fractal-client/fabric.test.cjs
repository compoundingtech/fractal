const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { once } = require('node:events');
const http = require('node:http');
const ts = require('typescript');
const { WebSocketServer } = require('ws');

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'fractal-fabric-ts-'));
let native;
before(async () => {
    fs.writeFileSync(path.join(root, 'package.json'), '{"type":"module"}');
    for (const name of ['Models.generated', 'Client.generated']) {
        const source = fs.readFileSync(path.join(__dirname, `${name}.ts`), 'utf8');
        const output = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020, rewriteRelativeImportExtensions: true } }).outputText;
        fs.writeFileSync(path.join(root, `${name}.js`), output);
    }
    fs.mkdirSync(path.join(root, 'node_modules'));
    fs.symlinkSync(path.dirname(require.resolve('ws/package.json')), path.join(root, 'node_modules/ws'), 'dir');
    const helper = fs.readFileSync(path.join(__dirname, 'fabric.mjs'), 'utf8').replace('./Client.generated.ts', './Client.generated.js');
    fs.writeFileSync(path.join(root, 'fabric.mjs'), helper);
    native = await import(pathToFileURL(path.join(root, 'fabric.mjs')).href);
});
after(() => fs.rmSync(root, { recursive: true, force: true }));

const fixture = (name, peers, socket, refusal = false) => {
    const program = path.join(root, `fabric-${name}`);
    const calls = path.join(root, `calls-${name}`);
    fs.writeFileSync(program, `#!${process.execPath}\nconst fs=require('node:fs'); const args=process.argv.slice(2); fs.appendFileSync(${JSON.stringify(calls)},JSON.stringify(args)+'\\n'); if(args[0]==='peers') console.log(${JSON.stringify(JSON.stringify({ peers }))}); else if(${refusal}) {console.log('{"error":{"code":"grant_denied","class":"refusal"}}');process.exit(1);} else console.log(${JSON.stringify(JSON.stringify({ socket }))});\n`);
    // .cjs keeps the executable fixture independent of the helper's ESM package.
    const executable = `${program}.cjs`;
    fs.renameSync(program, executable);
    fs.chmodSync(executable, 0o700);
    return { program: executable, calls };
};

test('native Fabric parsing, roundtrip and pre-dial routing guards', async () => {
    const { FabricEndpoint, createFabricTransport } = native;
    for (const target of ['DEVBOX', 'dev-box.local', 'a'.repeat(64)]) {
        const endpoint = `fabric://${target}/st3-client-v0`;
        assert.equal(FabricEndpoint.parse(endpoint).toString(), endpoint);
    }
    for (const endpoint of ['fabric:', 'fabric://box', 'fabric://box/other', 'fabric://user@box/st3-client-v0', 'fabric://box/st3-client-v0?token=x']) {
        assert.throws(() => FabricEndpoint.parse(endpoint));
    }
    const transport = await createFabricTransport('fabric://box/st3-client-v0', { program: '/not/a/fabric-program' });
    for (const route of ['/v1/status', '/v1/client/../admin', '/v1/client/%2e%2e/admin', '/v1/client/%252e%252e/admin', '/v1/client/a%2f..', '/v1/client/%00', '/v1/client/%zz', '/v1/healthz', '/v1/health/live']) {
        await assert.rejects(transport.fetchImpl(`http://fabric.local${route}`), error => error.code === 'invalid-path');
        assert.throws(() => transport.socket(`ws://fabric.local${route}`, [], {}), error => error.code === 'invalid-path');
    }
    for (const route of ['/v1/client/agents', '/v1/client/conversations/a/stream?after=a%2Fb', '/v1/client/agent%2Fdemo/file%2Emd', '/v1/health']) {
        FabricEndpoint.validatePath(route);
    }
});

test('trusted names resolve case-insensitively and ambiguity/refusal fail closed', async () => {
    const socket = path.join(root, 'unused.sock');
    const unique = fixture('unique', [{ node: 'node-a', name: 'Box' }], socket);
    assert.equal(await native.FabricEndpoint.parse('fabric://bOX/st3-client-v0').dial(unique), socket);
    const calls = fs.readFileSync(unique.calls, 'utf8').trim().split('\n').map(JSON.parse);
    assert.deepEqual(calls, [['peers', '--json'], ['dial', 'node-a', 'st3-client-v0', '--json']]);
    const ambiguous = fixture('ambiguous', [{ node: 'node-a', name: 'Box' }, { node: 'node-b', name: 'BOX' }], socket);
    await assert.rejects(native.FabricEndpoint.parse('fabric://box/st3-client-v0').dial(ambiguous), error => error.code === 'ambiguous-peer');
    assert.equal(fs.readFileSync(ambiguous.calls, 'utf8').trim().split('\n').length, 1);
    const refused = fixture('refused', [], socket, true);
    await assert.rejects(native.FabricEndpoint.parse(`fabric://${'a'.repeat(64)}/st3-client-v0`).dial(refused), error => error.code === 'grant_denied');
    const hex = 'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a';
    const base32 = '25NJQAMCWEFLPVKL73J4SZAHHIHOC4XT3KTCGJNPAINGR5YHKENA';
    for (const [index, target] of [hex, base32, base32.toLowerCase()].entries()) {
        const direct = fixture(`node-${index}`, [], socket);
        assert.equal(await native.FabricEndpoint.parse(`fabric://${target}/st3-client-v0`).dial(direct), socket);
        assert.deepEqual(JSON.parse(fs.readFileSync(direct.calls, 'utf8').trim()), ['dial', hex, 'st3-client-v0', '--json']);
    }
});

test('fake Fabric carries authenticated HTTP and every client WebSocket stream', { timeout: 10000 }, async t => {
    const socket = path.join(root, 'client.sock');
    const snapshot = { id: 'snapshot/test', host_id: 'host/test', store_index: 1, projection_version: 'client-projection.v0', created_at: '2026-09-20T00:00:00Z' };
    const envelope = value => ({ api_version: 'st3.client.v0', request_id: 'request/test', snapshot, value });
    const requests = [];
    const uploadBytes = Buffer.from([0, 137, 80, 78, 71, 255, 13, 10]);
    const uploads = [];
    const server = http.createServer((request, response) => {
        requests.push(request);
        assert.equal(request.headers.authorization, 'Bearer paired-token');
        assert.equal(request.headers['x-st3-person'], undefined);
        response.setHeader('content-type', 'application/json');
        if (request.url === '/v1/client/blobs') {
            assert.equal(request.method, 'POST');
            assert.equal(request.headers['content-type'], 'image/png');
            const chunks = [];
            request.on('data', chunk => chunks.push(chunk));
            request.on('end', () => {
                const bytes = Buffer.concat(chunks);
                assert.deepEqual(bytes, uploadBytes);
                uploads.push(bytes);
                response.end(JSON.stringify(envelope({ blob: 'blob/test', media_type: 'image/png', sha256: 'a'.repeat(64), size: bytes.length })));
            });
            return;
        }
        response.end(JSON.stringify(envelope({ capabilities: [] })));
    });
    const sockets = new WebSocketServer({ noServer: true });
    const upgraded = [];
    server.on('upgrade', (request, stream, head) => {
        assert.equal(request.headers.authorization, 'Bearer paired-token');
        upgraded.push(request.url);
        sockets.handleUpgrade(request, stream, head, connection => {
            if (request.url.includes('/collections/')) {
                connection.once('message', data => {
                    assert.equal(JSON.parse(data.toString()).kind, 'subscribe');
                    connection.send(JSON.stringify({ kind: 'resync', id: 'agents', reason: 'test' }));
                });
            } else if (request.url.includes('/conversations/')) {
                connection.send(JSON.stringify(envelope({ kind: 'conversation-changes' })));
            } else {
                connection.send(JSON.stringify(envelope({ kind: 'terminal-screen' })));
            }
        });
    });
    server.listen(socket);
    await once(server, 'listening');
    t.after(() => { for (const client of sockets.clients) client.terminate(); sockets.close(); server.closeAllConnections(); server.close(); });
    const fabric = fixture('roundtrip', [{ node: 'node-a', name: 'Box' }], socket);
    const client = await native.createFabricClient({ endpoint: 'fabric://BOX/st3-client-v0', program: fabric.program, credential: () => 'paired-token' });
    assert.deepEqual((await client.capabilities()).value, { capabilities: [] });
    for (const body of [
        new Blob([uploadBytes], { type: 'image/png' }),
        Uint8Array.from(uploadBytes).buffer,
        Uint8Array.from(uploadBytes),
    ]) {
        const uploaded = await client.uploadBlob(body, 'image/png');
        assert.equal(uploaded.value.size, uploadBytes.length);
    }
    assert.equal(uploads.length, 3);
    const httpOnly = path.join(root, 'http-only');
    fs.mkdirSync(path.join(httpOnly, 'node_modules/ws'), { recursive: true });
    fs.writeFileSync(path.join(httpOnly, 'node_modules/ws/package.json'), '{"type":"module","exports":"./index.js"}');
    fs.writeFileSync(path.join(httpOnly, 'node_modules/ws/index.js'), 'throw new Error("HTTP must not load optional ws");');
    const helper = fs.readFileSync(path.join(__dirname, 'fabric.mjs'), 'utf8').replace('./Client.generated.ts', '../Client.generated.js');
    fs.writeFileSync(path.join(httpOnly, 'fabric.mjs'), helper);
    const noWebSocket = await import(pathToFileURL(path.join(httpOnly, 'fabric.mjs')).href);
    const httpClient = await noWebSocket.createFabricClient({ endpoint: 'fabric://box/st3-client-v0', program: fabric.program, credential: () => 'paired-token' });
    assert.deepEqual((await httpClient.capabilities()).value, { capabilities: [] });
    const traced = client.withTraceContext(() => undefined);
    let received;
    const collectionFrame = new Promise(resolve => { received = resolve; });
    const collection = await traced.collectionStream({ onFrame: received });
    collection.subscribe('agents', 'agents', 10);
    await collectionFrame;
    collection.close();
    const conversationFrame = new Promise(resolve => { received = resolve; });
    const conversation = await client.conversationStream('session/test', { onChange: received });
    await conversationFrame;
    conversation.close();
    const terminalFrame = new Promise(resolve => { received = resolve; });
    const terminal = await client.terminalStream('terminal/test', { streamCapability: 'cap', onScreen: received });
    await terminalFrame;
    terminal.close();
    assert.equal(requests.length, 5);
    assert.equal(upgraded.length, 3);
});
