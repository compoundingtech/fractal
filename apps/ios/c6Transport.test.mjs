import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createRequire } from 'node:module';
import { createHash, generateKeyPairSync, sign, verify } from 'node:crypto';
import { FractalClient } from '../../clients/typescript/fractal-client/Client.generated.ts';
import { conversationEntries, messageSubject, signedMessageFields, signedBytes, signatureParameter } from '@fractal/views';
import { C6Recorder } from './c6Recorder.ts';
import { sendSignedMessage } from './c6MessageSend.ts';
import { resendBoundMessage } from './messageSend.ts';

const require = createRequire(import.meta.url);
const { AbortController: RNAbortController } = require('abort-controller/dist/abort-controller');
const capabilities = require('../../docs/st3/client-v0/fixtures/capabilities.json');
const fence = { snapshot_id: capabilities.snapshot.id, subject_revisions: {} };
const to = 'agent/example/c6', actor = 'person/c6';
const { privateKey, publicKey } = generateKeyPairSync('ec', { namedCurve: 'prime256v1' });

const rn = t => {
  const original = globalThis.AbortController;
  globalThis.AbortController = RNAbortController;
  t.after(() => { globalThis.AbortController = original; });
};
const response = value => new Response(JSON.stringify({ ...capabilities, value }), { headers: { 'Content-Type': 'application/json' } });
const signedParameters = (id, content) => {
  const message = {
    subject: messageSubject(createHash('sha256').update(id).digest('hex')), signer: actor,
    fields: signedMessageFields({ from: actor, in_reply_to: null, session_id: null, tags: [], title: null, to }, content),
    key: 'test-key', chain: [], nonce: Buffer.from(id).toString('base64url'), signedAt: 1,
  };
  const signature = sign('sha256', Buffer.from(signedBytes(message)), { key: privateKey, dsaEncoding: 'ieee-p1363' }).toString('base64url');
  assert(verify('sha256', Buffer.from(signedBytes(message)), { key: publicKey, dsaEncoding: 'ieee-p1363' }, Buffer.from(signature, 'base64url')));
  return { to, content, signature: signatureParameter(message, signature) };
};
const send = (client, hooks, content, id, parameters = signedParameters) => sendSignedMessage('tailscale-member', {
  client, c6Send: hooks, actionId: () => id, parameters: async action => parameters(action, content),
  fences: async () => fence, fence: async () => fence, originalAvailable: () => {}, wait: async () => {},
});
const fixture = (fetchImpl, timeoutMs = 1000) => {
  let id = 0;
  const identity = new FractalClient({ baseUrl: 'http://100.101.102.103:8790', credential: () => 'test-credential', fetchImpl });
  const recorder = new C6Recorder({
    now: () => 0, id: () => `transport-${++id}`, version: 'test', changed: () => {}, timeoutMs,
    snapshot: () => ({ identity, context: { route: 'tailscale', path: 'unknown' } }),
    readFabricPath: async () => assert.fail('Fabric is off'),
  });
  return { identity, recorder };
};
const executeStatus = async (_, hooks, sample) => {
  if (!sample.identity) throw new Error('C6 disconnected');
  const reply = await sample.identity.capabilities({ signal: hooks.signal });
  hooks.request(reply.request_id); hooks.ack();
};
const executeSend = async (nonce, hooks, sample) => {
  const content = `[C6:${nonce}] latency measurement`;
  const sent = await send(sample.identity, hooks, content, `action/${nonce}`);
  if (sent.how !== 'sent') throw new Error('C6 send failed');
};

test('C6 signed send+echo under the React Native AbortSignal polyfill', async t => {
  rn(t);
  let actions = 0, signatures = 0, closed = 0, onEntries;
  const { identity, recorder } = fixture(async (url, init) => {
    if (url.endsWith('/capabilities')) return response(capabilities.value);
    const action = JSON.parse(init.body);
    actions++;
    assert.equal(action.type, 'message.send');
    assert.equal(action.parameters.signature.format, 'fields-v1');
    signatures++;
    // The decoded timeline delivered by the conversation subscription, as in Fleet.
    onEntries({ items: [
      { id: `header-${actions}`, sequence: 1, revision: 1, timestamp: '2026-11-10T00:00:00Z', role: 'user', type: 'message', final: true, body: { message_id: `message/${actions}`, from: actor, to } },
      { id: `content-${actions}`, sequence: 2, revision: 1, timestamp: '2026-11-10T00:00:00Z', role: 'user', type: 'content', final: true, body: { media_type: 'text/plain', text: action.parameters.content } },
    ] });
    return response({ kind: 'action-result', action_id: action.id, operation_id: `operation/${actions}`, status: 'accepted', affected_ids: [], snapshot_id: fence.snapshot_id });
  });
  const feed = { followConversation(target, handlers) { assert.equal(target, to); onEntries = handlers.onEntries; return { close() { closed++; onEntries = undefined; } }; } };
  await recorder.run('send-echo', async (nonce, hooks, sample) => {
    if (!sample.identity) throw new Error('C6 disconnected');
    const names = new Map([[actor, 'you']]);
    const follow = feed.followConversation(to, {
      onEntries: frame => {
        for (const entry of conversationEntries(frame.items, names)) {
          if (entry.body.kind === 'mail') recorder.observe(entry.id, entry.body.text, entry.body.from === 'you');
        }
      },
      onIssue: () => {}, onReplaced: () => recorder.cancel(),
    });
    hooks.cleanup(() => follow.close());
    const sent = await send(identity, hooks, `[C6:${nonce}] latency measurement`, `action/${nonce}`);
    if (sent.how !== 'sent') throw new Error('C6 send failed');
  });
  assert.equal(actions, 10); assert.equal(signatures, 10); assert.equal(closed, 10);
  assert(recorder.rows.every(row => row.status === 'ok' && row.errorName === null && row.requestId && row.messageId && row.ackMs === 0 && row.echoMs === 0));
});

for (const operation of ['status', 'send-echo']) {
  for (const phase of ['pre-abort', 'in-flight']) {
    for (const mode of ['cancelled', 'timeout']) {
      test(`C6 ${operation} ${phase} ${mode} under the React Native AbortSignal polyfill`, async t => {
        rn(t);
        let fetches = 0, requests = 0;
        let recorder;
        const made = fixture(async (url, init) => {
          fetches++;
          if (operation === 'send-echo' && url.endsWith('/capabilities')) return response(capabilities.value);
          requests++;
          return new Promise((_, reject) => {
            init.signal.addEventListener('abort', () => reject(Object.assign(new Error('transport aborted'), { name: 'AbortError' })), { once: true });
            if (mode === 'cancelled') recorder.cancel();
          });
        }, 1);
        recorder = made.recorder;
        const execute = operation === 'status' ? executeStatus : executeSend;
        await recorder.run(operation, async (nonce, hooks, sample) => {
          if (phase === 'pre-abort') {
            if (mode === 'cancelled') recorder.cancel();
            else {
              // Let the recorder's own deadline abort before entering the request.
              await new Promise(resolve => hooks.signal.addEventListener('abort', resolve, { once: true }));
            }
          }
          await execute(nonce, hooks, sample);
        });
        assert.equal(recorder.rows.length, 10);
        assert(recorder.rows.every(row => row.status === mode && row.errorName === null));
        if (phase === 'pre-abort') assert.equal(fetches, 0);
        else assert.equal(requests, mode === 'cancelled' ? 1 : 10);
      });
    }
  }
}

test('C6 signed send guards after signing and before a bound fence retry accept RN signals', async t => {
  rn(t);
  let fetches = 0, signs = 0;
  const { identity } = fixture(async url => {
    fetches++;
    if (url.endsWith('/capabilities')) return response(capabilities.value);
    throw new TypeError('connection lost');
  });
  const controller = new RNAbortController();
  const hooks = { signal: controller.signal, request: () => {}, ack: () => {}, cleanup: () => {} };
  const aborted = await send(identity, hooks, 'hello', 'action/aborted', (id, content) => { signs++; controller.abort(); return signedParameters(id, content); });
  assert.equal(aborted.how, 'failed'); assert.equal(aborted.error.name, 'AbortError');
  assert.equal(signs, 1); assert.equal(fetches, 0);
  const retryController = new RNAbortController();
  const pending = await send(identity, { ...hooks, signal: retryController.signal }, 'hello', 'action/lost');
  assert.equal(pending.how, 'unconfirmed');
  const before = fetches;
  retryController.abort();
  const retried = await resendBoundMessage(pending.request);
  assert.equal(retried.how, 'unconfirmed');
  assert.equal(fetches, before);
});

test('C6 failed rows preserve the thrown name under the React Native AbortSignal polyfill', async t => {
  rn(t);
  const { recorder } = fixture(async () => { throw new TypeError('do not export this message'); });
  await recorder.run('status', executeStatus);
  assert(recorder.rows.every(row => row.status === 'failed' && row.errorName === 'TypeError'));
  assert(!recorder.export().includes('do not export this message'));
});
