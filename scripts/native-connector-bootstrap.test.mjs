import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs/promises';
import http from 'node:http';
import { stripTypeScriptTypes } from 'node:module';
import os from 'node:os';
import path from 'node:path';
import { Duplex } from 'node:stream';
import test from 'node:test';

// The published assets are standalone, so exercise each embedded implementation rather
// than introducing a runtime module import that hook publication cannot ship.
const loadRegistration = async (asset) => {
  const source = await fs.readFile(new URL(`../crates/${asset}`, import.meta.url), 'utf8');
  const start = source.indexOf('const registerConnector = (');
  assert.notEqual(start, -1);
  const end = source.indexOf('\n};', start) + 3;
  const implementation = stripTypeScriptTypes(source.slice(start, end));
  return new Function('http', 'Duplex', 'HELLO_TIMEOUT_MS',
    `${implementation}\nreturn registerConnector;`)(http, Duplex, 5000);
};

for (const crate of ['fractal-drivers', 'fractal-daemon']) {
  for (const harness of ['omp', 'pi']) {
    const asset = `${crate}/hooks/${harness}-channel.ts`;
    for (const status of [204, 403]) {
      test(`${asset}: ${status} gates the actual spawned child`,
        { skip: process.platform === 'win32', timeout: 10000 }, async (t) => {
          const register = await loadRegistration(asset);
          const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'native-bootstrap-'));
          t.after(() => fs.rm(directory, { recursive: true, force: true }));
          const socket = path.join(directory, 'daemon.sock');
          const server = http.createServer();
          t.after(() => new Promise((resolve) => server.close(resolve)));
          server.listen(socket);
          await once(server, 'listening');
          const child = childProcess.spawn(process.execPath, ['-e', `
            const fs = require('node:fs');
            const byte = Buffer.alloc(1);
            if (fs.readSync(3, byte, 0, 1, null) !== 1 || byte[0] !== 1) process.exit(2);
            process.stdout.write('ready');
          `], { stdio: ['pipe', 'pipe', 'inherit', 'pipe'] });
          t.after(() => { if (child.exitCode === null && child.signalCode === null) child.kill(); });
          let output = '';
          child.stdout.setEncoding('utf8').on('data', (chunk) => { output += chunk; });
          const exited = once(child, 'close');
          const requestReceived = once(server, 'request');
          const facts = { subject: 'host.worker', runtime_incarnation: 'runtime-1',
            harness, session: 'session-1', sequence: 7 };
          register(child, socket, facts);
          const [request, response] = await requestReceived;
          let body = '';
          for await (const chunk of request) body += chunk;
          assert.equal(request.method, 'POST');
          assert.equal(request.url, '/v1/native-connectors/bootstrap');
          assert.deepEqual(JSON.parse(body), { ...facts, connector_pid: child.pid });
          assert.equal(output, '', 'connector cannot proceed while registration is pending');
          response.writeHead(status).end();
          const [code, signal] = await exited;
          if (status === 204) {
            assert.equal(code, 0);
            assert.equal(signal, null);
            assert.equal(output, 'ready');
          } else {
            assert.notEqual(code, 0);
            assert.equal(output, '', 'refused registration must never release the connector');
          }
        });
    }
  }
}
