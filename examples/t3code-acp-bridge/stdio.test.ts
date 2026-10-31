import { execFile } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { Schema } from 'effect'
import { expect, it } from 'vitest'

const decode = Schema.decodeUnknownSync(Schema.fromJsonString(Schema.Struct({
  jsonrpc: Schema.Literal('2.0'),
  id: Schema.NullOr(Schema.Union([Schema.String, Schema.Number])),
  result: Schema.optional(Schema.Unknown),
  error: Schema.optional(Schema.Struct({ code: Schema.Number, message: Schema.String })),
})))

it('runs as a Node stdio command with JSON-RPC-only stdout and exits on EOF', async () => {
  // This subprocess is the bridge, not an agent harness. initialize must not dial a daemon.
  const stdout = await new Promise<string>((resolve, reject) => {
    const child = execFile(process.execPath, [fileURLToPath(new URL('./stdio.ts', import.meta.url))], {
      env: { ...process.env, FRACTAL_ACP_URL: 'http://unreachable.invalid' },
    }, (error, stdout) => error === null ? resolve(stdout) : reject(error))
    child.stdin?.end('not-json\n{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":2}}\n{"jsonrpc":"2.0","id":2,"method":"session/fork","params":{}}\n')
  })
  const messages = stdout.trim().split('\n').map(line => decode(line))
  expect(messages).toHaveLength(3)
  expect(messages.find(message => message.id === null)?.error?.code).toBe(-32700)
  expect(messages.find(message => message.id === 1)?.result).toMatchObject({ protocolVersion: 1, agentCapabilities: { loadSession: true } })
  expect(messages.find(message => message.id === 2)?.error?.code).toBe(-32601)
})
