import { spawn, type ChildProcess } from 'node:child_process'
import { once } from 'node:events'
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, it } from 'vitest'

const app = fileURLToPath(new URL('../', import.meta.url))
const token = 'synthetic-preview-credential'
const authorization = `Bearer ${token}`

it.each([token, authorization])('keeps preview credentials out of the child environment and authorizes gateway requests (%#)', async (credential) => {
  const root = await mkdtemp(join(tmpdir(), 'fractal-preview-test-'))
  const credentialFile = join(root, 'credential')
  const launcher = join(root, 'app/scripts/devnet/preview-launch.sh')
  const vite = join(root, 'app/node_modules/.bin/vite')
  const environmentFile = join(root, 'environment.json')
  const configFile = join(root, 'vite.config.mjs')
  const socketPath = join(root, 'gateway.sock')
  const received: (string | undefined)[] = []
  const gateway = createServer((request, response) => {
    received.push(request.headers.authorization)
    response.setHeader('content-type', 'application/json')
    response.end('{"paired":true}')
  })
  let child: ChildProcess | undefined
  let exited: Promise<unknown> | undefined
  try {
    // Recreate the launcher's app-local binary boundary without depending on a node linker layout.
    // The binary harness starts real Vite and the real gateway plugin through the public API.
    await mkdir(join(root, 'app/scripts/devnet'), { recursive: true })
    await mkdir(join(root, 'app/node_modules/.bin'), { recursive: true })
    await copyFile(join(app, 'scripts/devnet/preview-launch.sh'), launcher)
    await writeFile(vite, `#!/usr/bin/env node
      import { createServer } from ${JSON.stringify(import.meta.resolve('vite'))}
      const server = await createServer({
        configFile: process.env.WF_VITE_CONFIG,
        configLoader: 'runner',
        server: { host: '127.0.0.1', port: 0, strictPort: true },
      })
      await server.listen()
    `, { mode: 0o700 })
    await writeFile(credentialFile, `${credential}\n`, { mode: 0o600 })
    await writeFile(configFile, `
      import { writeFileSync } from 'node:fs'
      import { webfractalGateway } from ${JSON.stringify(join(app, 'scripts/vite-gateway.mjs'))}
      export default {
        root: ${JSON.stringify(root)},
        logLevel: 'silent',
        plugins: [webfractalGateway(), {
          name: 'preview-environment-proof',
          configureServer(vite) {
            vite.httpServer.once('listening', () => {
              writeFileSync(${JSON.stringify(environmentFile)}, JSON.stringify(process.env), { mode: 0o600 })
              console.log('PREVIEW_PORT=' + vite.httpServer.address().port)
            })
          },
        }],
        server: { hmr: false, ws: false },
      }
    `)
    gateway.listen(socketPath)
    await once(gateway, 'listening')
    child = spawn('bash', [launcher], {
      // Only synthetic configuration reaches this subprocess, never the test runner's credentials.
      env: {
        PATH: process.env.PATH,
        DEV_EDGE_URL: 'https://preview.example.test',
        PORT: '0',
        WF_VITE_CONFIG: configFile,
        WF_ST_GATEWAY: `unix:${socketPath}`,
        WF_ST_CREDENTIAL_FILE: credentialFile,
        // A legacy caller must not accidentally leak this into Vite either.
        WF_ST_AUTHORIZATION: authorization,
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    exited = once(child, 'exit')
    const ready = Promise.withResolvers<number>()
    const deadline = AbortSignal.timeout(8000)
    deadline.addEventListener('abort', () => ready.reject(new Error('Preview did not start')), { once: true })
    let output = ''
    child.stdout!.on('data', (chunk) => {
      output += chunk.toString()
      const match = /PREVIEW_PORT=(\d+)/.exec(output)
      if (match) ready.resolve(Number(match[1]))
    })
    child.once('error', ready.reject)
    child.once('exit', () => ready.reject(new Error('Preview exited before listening')))
    const port = await ready.promise
    const env = JSON.parse(await readFile(environmentFile, 'utf8')) as Record<string, string>
    expect(env.WF_ST_CREDENTIAL_FILE).toBe(credentialFile)
    expect(env.WF_ST_AUTHORIZATION).toBeUndefined()
    expect(Object.values(env).some(value => value.includes(token) || /Bearer\s/i.test(value))).toBe(false)
    if (process.platform === 'linux') {
      const environment = await readFile(`/proc/${child.pid}/environ`, 'utf8')
      expect(environment.includes(token) || /Bearer\s/i.test(environment)).toBe(false)
    }
    const response = await fetch(`http://127.0.0.1:${port}/v1/client/status`)
    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({ paired: true })
    expect(received).toEqual([authorization])
  } finally {
    if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGTERM')
    await exited
    gateway.closeAllConnections()
    const closed = Promise.withResolvers<void>()
    gateway.close(() => closed.resolve())
    await closed.promise
    await rm(root, { recursive: true, force: true })
  }
})
