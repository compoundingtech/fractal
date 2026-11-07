import { createInterface } from 'node:readline'
import { once } from 'node:events'
import { Effect, Schema } from 'effect'
import { makeBackend } from './backend.ts'
import { AcpConnection, RpcError, type RpcMessage } from './protocol.ts'

const decode = Schema.decodeUnknownSync(Schema.fromJsonString(Schema.Unknown))
const encode = Schema.encodeSync(Schema.fromJsonString(Schema.Unknown))

// stdout is exclusively the ACP JSON-RPC stream. Diagnostics must use stderr.
const main = Effect.tryPromise({
  try: async () => {
    let output = Promise.resolve()
    const send = (message: RpcMessage) => {
      const line = `${encode(message)}\n`
      output = output.then(async () => {
        if (!process.stdout.write(line)) await once(process.stdout, 'drain')
      })
    }
    const connection = new AcpConnection(makeBackend(), send)
    const input = createInterface({ input: process.stdin, crlfDelay: Infinity })
    const requests = new Set<Promise<void>>()
    try {
      for await (const line of input) {
        let value: unknown
        try { value = decode(line) } catch {
          send({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'Parse error' } })
          continue
        }
        // Do not await: requests and reverse permission responses share this input stream.
        const request = connection.receive(value)
        requests.add(request)
        void request.finally(() => requests.delete(request))
      }
    } finally {
      connection.close()
      // EOF detaches, not cancels: daemon ownership survives the UI process.
      await Promise.all(requests)
      await output
    }
  },
  catch: cause => new RpcError({ code: -32603, message: cause instanceof Error ? cause.message : String(cause) }),
})

Effect.runPromise(main).catch(cause => {
  process.stderr.write(`${String(cause)}\n`)
  process.exitCode = 1
})
