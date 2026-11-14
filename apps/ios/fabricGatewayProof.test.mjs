import assert from 'node:assert/strict';
import { test } from 'node:test';
import { verifyUnpairedFabricGateway } from './fabricGatewayProof.ts';
const denied = { api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', code: 'forbidden' };
test('proof checks a protected route without credentials, independently of public discovery', async () => {
  await verifyUnpairedFabricGateway('http://127.0.0.1:1234', 'fractal-ios 0.1.0+test', async (url, init) => {
    assert.equal(url, 'http://127.0.0.1:1234/v1/client/agents?limit=1');
    assert.equal(init.headers.Authorization, undefined);
    assert.equal(init.headers['x-st3-client'], 'fractal-ios 0.1.0+test');
    return Response.json(denied, { status: 403 });
  });
});
test('public discovery, successful access, and unrelated proxy refusal cannot establish gateway protection', async () => {
  for (const [status, payload] of [[200, { api_version: 'st3.client.v0', capabilities: [] }], [200, { api_version: 'st3.client.v0', value: {} }], [403, { message: 'proxy denial' }]]) {
    await assert.rejects(verifyUnpairedFabricGateway('http://127.0.0.1:1234', 'fractal-ios 0.1.0+test', async () => Response.json(payload, { status })), /did not refuse/);
  }
});
test('fabric proof accepts both API and error namespaces independently, but not unrelated versions', async () => {
  for (const api_version of ['st3.client.v0', 'fractal.client.v0', 'unrelated.client.v0']) {
    for (const error_version of ['st3.client.error.v0', 'fractal.client.error.v0', 'unrelated.client.error.v0']) {
      const proof = verifyUnpairedFabricGateway('http://127.0.0.1:1234', 'test', async () => Response.json({ ...denied, api_version, error_version }, { status: 403 }));
      if (api_version.startsWith('unrelated') || error_version.startsWith('unrelated')) await assert.rejects(proof, /did not refuse/);
      else await proof;
    }
  }
});
