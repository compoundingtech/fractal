// Node/Bun-only entrypoint. Browser and React Native clients never import Node built-ins.
import { execFile } from 'node:child_process';
import { request } from 'node:http';
import { isAbsolute } from 'node:path';
import { Readable } from 'node:stream';
import { promisify } from 'node:util';
import { FractalClient } from './Client.generated.ts';

const execute = promisify(execFile);
const protocol = 'st3-client-v0';
const origin = 'http://fabric.local';

// Recognize canonical NodeID encodings; Fabric validates the curve point on dial.
// Unlike Rust's Iroh parser, an invalid curve key shaped like a NodeID fails closed
// at the daemon rather than falling back to a trusted peer with that literal name.
const nodeId = target => {
    if (/^[a-f0-9]{64}$/.test(target)) return target;
    if (!/^[a-z2-7]{51}[aq]$/i.test(target)) return undefined;
    const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
    let accumulator = 0, bits = 0;
    const bytes = [];
    for (const character of target.toUpperCase()) {
        accumulator = (accumulator << 5) | alphabet.indexOf(character);
        bits += 5;
        if (bits >= 8) {
            bits -= 8;
            bytes.push((accumulator >> bits) & 255);
            accumulator &= (1 << bits) - 1;
        }
    }
    return Buffer.from(bytes).toString('hex');
};

export class FabricTransportError extends Error {
    constructor(code, message) {
        super(message);
        this.name = 'FabricTransportError';
        this.code = code;
    }
}

export class FabricEndpoint {
    #target;
    constructor(value) {
        const match = /^fabric:\/\/([A-Za-z0-9_.-]+)\/st3-client-v0$/.exec(value);
        if (!match || match[1] === '.' || match[1] === '..') {
            throw new FabricTransportError('invalid-endpoint', 'Expected fabric://<NodeID-or-name>/st3-client-v0');
        }
        this.#target = match[1];
    }
    static parse(value) { return new FabricEndpoint(value); }
    toString() { return `fabric://${this.#target}/${protocol}`; }
    static validatePath(path) {
        const route = path.split('?')[0];
        const allowed = value => value.startsWith('/v1/client/') || value === '/v1/health';
        if (/[\x00-\x1f\x7f\\#]/.test(path) || !allowed(route)) {
            throw new FabricTransportError('invalid-path', 'Path is not allowed over Fabric');
        }
        let decoded;
        try { decoded = decodeURIComponent(route); } catch {
            throw new FabricTransportError('invalid-path', 'Malformed encoded Fabric route');
        }
        if (!allowed(decoded) || /[\x00-\x1f\x7f\\%?#]/.test(decoded)
            || decoded.split('/').some(part => part === '.' || part === '..')) {
            throw new FabricTransportError('invalid-path', 'Encoded routing escapes are not allowed over Fabric');
        }
    }
    async dial({ program = 'fabric', signal } = {}) {
        const run = async args => {
            signal?.throwIfAborted();
            try {
                const { stdout } = await execute(program, [...args, '--json'], { timeout: 10000, maxBuffer: 1024 * 1024, signal });
                return JSON.parse(stdout);
            } catch (error) {
                if (signal?.aborted) throw signal.reason;
                let code = 'fabric-command-failed';
                try {
                    const answer = JSON.parse(error.stdout);
                    if (typeof answer.error?.code === 'string') code = answer.error.code;
                } catch { /* Invalid command output is a transport failure. */ }
                throw new FabricTransportError(code, 'The local Fabric daemon could not open the client transport');
            }
        };
        let node = nodeId(this.#target);
        if (node === undefined) {
            node = this.#target;
            const answer = await run(['peers']);
            if (!Array.isArray(answer.peers)) throw new FabricTransportError('invalid-peers', 'Fabric peers returned no peer list');
            const peers = answer.peers.filter(peer => typeof peer.node === 'string' && peer.node.length > 0);
            const exact = peers.find(peer => peer.node === node);
            const matches = exact ? [exact] : peers.filter(peer => typeof peer.name === 'string' && peer.name.toLowerCase() === node.toLowerCase());
            if (matches.length !== 1) throw new FabricTransportError(matches.length ? 'ambiguous-peer' : 'unknown-peer', 'Choose one unique trusted Fabric peer name or a NodeID');
            node = matches[0].node;
        }
        const answer = await run(['dial', node, protocol]);
        if (typeof answer.socket !== 'string' || !isAbsolute(answer.socket)) {
            throw new FabricTransportError('invalid-socket', 'Fabric dial returned no absolute Unix socket path');
        }
        return answer.socket;
    }
}

const requestPath = (value, websocket = false) => {
    const prefix = websocket ? 'ws://fabric.local' : origin;
    if (typeof value !== 'string' || !value.startsWith(`${prefix}/`)) {
        throw new FabricTransportError('invalid-path', 'Native Fabric transport cannot request another origin');
    }
    const path = value.slice(prefix.length);
    FabricEndpoint.validatePath(path);
    return path;
};

/** Create native transports separately so tests and advanced callers can use the same boundary. */
export const createFabricTransport = async (value, { program = 'fabric' } = {}) => {
    const endpoint = FabricEndpoint.parse(value);
    const fetchImpl = async (input, init = {}) => {
        const path = requestPath(input);
        const signal = init.signal ?? undefined;
        signal?.throwIfAborted();
        const socketPath = await endpoint.dial({ program, signal });
        signal?.throwIfAborted();
        return new Promise((resolve, reject) => {
            const headers = Object.fromEntries(new Headers(init.headers));
            const pending = request({ socketPath, path, method: init.method ?? 'GET', headers, signal }, response => {
                const resultHeaders = new Headers();
                for (let index = 0; index < response.rawHeaders.length; index += 2) {
                    resultHeaders.append(response.rawHeaders[index], response.rawHeaders[index + 1]);
                }
                const body = [204, 205, 304].includes(response.statusCode) ? null : Readable.toWeb(response);
                resolve(new Response(body, { status: response.statusCode, statusText: response.statusMessage, headers: resultHeaders }));
            });
            pending.on('error', reject);
            pending.end(init.body);
        });
    };
    const socket = (url, protocols, headers) => {
        const path = requestPath(url, true);
        let connection;
        let closed = false;
        const controller = new AbortController();
        const surface = {
            onopen: null, onmessage: null, onclose: null, onerror: null,
            send(data) {
                if (!connection || connection.readyState !== 1) {
                    throw new FabricTransportError('socket-not-open', 'Wait for the native Fabric WebSocket to open before sending');
                }
                connection.send(data);
            },
            close(code = 1000, reason = '') {
                if (closed) return;
                closed = true;
                controller.abort();
                if (connection?.readyState === 1) connection.close(code, reason);
                else connection?.terminate();
            },
        };
        (async () => {
            // Optional for HTTP-only native clients and all browser installations.
            const { default: WebSocket } = await import('ws').catch(() => {
                throw new FabricTransportError('missing-websocket-runtime', 'Install ws@8.22.0 to use native Fabric streams');
            });
            const socketPath = await endpoint.dial({ program, signal: controller.signal });
            return { WebSocket, socketPath };
        })().then(({ WebSocket, socketPath }) => {
            if (closed) return;
            connection = new WebSocket(`ws+unix://${socketPath}:${path}`, protocols, { headers, handshakeTimeout: 10000 });
            connection.on('open', () => { if (!closed) surface.onopen?.(); });
            connection.on('message', data => { if (!closed) surface.onmessage?.({ data: data.toString() }); });
            connection.on('close', (code, reason) => {
                if (closed) return;
                closed = true;
                surface.onclose?.({ code, reason: reason.toString() });
            });
            connection.on('error', error => { if (!closed) surface.onerror?.(error); });
        }).catch(error => { if (!closed) { closed = true; surface.onerror?.(error); } });
        return surface;
    };
    return { fetchImpl, socket };
};

export const createFabricClient = async ({ endpoint, program, ...options }) => {
    const transport = await createFabricTransport(endpoint, { program });
    return new FractalClient({ ...options, ...transport, baseUrl: origin });
};
