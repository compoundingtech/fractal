import { isAbsolute } from 'node:path'
import { Schema } from 'effect'
import type { BridgeBackend, BridgePermission, BridgeUpdate } from './backend.ts'

export class RpcError extends Schema.TaggedErrorClass<RpcError>()('RpcError', {
  code: Schema.Number,
  message: Schema.String,
}) {}

const object = (value: unknown): Record<string, unknown> => {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new RpcError({ code: -32602, message: 'Expected an object' })
  }
  return value as Record<string, unknown>
}
const string = (value: unknown): string => {
  if (typeof value !== 'string' || value.length === 0) {
    throw new RpcError({ code: -32602, message: 'Expected a non-empty string' })
  }
  return value
}
export type RpcMessage = Record<string, unknown>
type PendingPermission = { resolve: (result: unknown) => void; reject: (error: unknown) => void }

/** One connection; dispatch remains concurrent so cancel/permission replies can unblock a prompt. */
export class AcpConnection {
  private initialized = false
  private used = false
  private nextPermission = 0
  private readonly sessions = new Set<string>()
  private readonly turns = new Map<string, AbortController>()
  private readonly permissions = new Map<string, PendingPermission>()

  private readonly backend: BridgeBackend
  private readonly send: (message: RpcMessage) => void

  constructor(backend: BridgeBackend, send: (message: RpcMessage) => void) {
    this.backend = backend
    this.send = send
  }

  private update(sessionId: string, update: BridgeUpdate) {
    this.send({ jsonrpc: '2.0', method: 'session/update', params: { sessionId, update } })
  }

  private async permission(sessionId: string, request: BridgePermission, signal: AbortSignal): Promise<'allow' | 'reject'> {
    if (signal.aborted) return 'reject'
    const id = `fractal-permission-${++this.nextPermission}`
    const result = await new Promise<unknown>((resolve, reject) => {
      const abort = () => { this.permissions.delete(id); resolve({ outcome: { outcome: 'cancelled' } }) }
      this.permissions.set(id, {
        resolve: value => { signal.removeEventListener('abort', abort); resolve(value) },
        reject: error => { signal.removeEventListener('abort', abort); reject(error) },
      })
      signal.addEventListener('abort', abort, { once: true })
      this.send({ jsonrpc: '2.0', id, method: 'session/request_permission', params: { sessionId, ...request } })
    })
    const outcome = object(object(result).outcome)
    if (outcome.outcome === 'cancelled') return 'reject'
    if (outcome.outcome !== 'selected') throw new RpcError({ code: -32602, message: 'Invalid permission outcome' })
    const selected = request.options.find(option => option.optionId === outcome.optionId)
    if (selected === undefined) throw new RpcError({ code: -32602, message: 'Unknown permission option' })
    return selected.kind === 'allow_once' ? 'allow' : 'reject'
  }

  private session(params: Record<string, unknown>) {
    const sessionId = string(params.sessionId)
    if (!this.sessions.has(sessionId)) throw new RpcError({ code: -32602, message: 'Load the session before using it' })
    return sessionId
  }

  private async dispatch(method: string, params: Record<string, unknown>): Promise<unknown> {
    if (method === 'initialize') {
      if (this.used) throw new RpcError({ code: -32600, message: 'Initialize must precede session operations' })
      if (!Number.isSafeInteger(params.protocolVersion) || Number(params.protocolVersion) < 1) {
        throw new RpcError({ code: -32602, message: 'protocolVersion must be a positive integer' })
      }
      this.initialized = true
      // t3code offers v2 preview; select the supported v1 shape, never claim v2 semantics.
      return { protocolVersion: 1, agentCapabilities: { loadSession: true, promptCapabilities: { image: false, audio: false, embeddedContext: false }, mcpCapabilities: { http: false, sse: false } }, agentInfo: { name: 'fractal-acp-spike', version: '0.1.0' }, authMethods: [] }
    }
    if (!this.initialized) throw new RpcError({ code: -32600, message: 'Initialize first' })
    this.used = true
    switch (method) {
      case 'session/new':
      case 'session/load': {
        const cwd = string(params.cwd)
        if (!isAbsolute(cwd)) throw new RpcError({ code: -32602, message: 'cwd must be absolute' })
        if (!Array.isArray(params.mcpServers) || params.mcpServers.length !== 0) {
          throw new RpcError({ code: -32602, message: 'This spike does not attach client MCP servers; use an empty mcpServers array' })
        }
        if (params.additionalDirectories !== undefined && (!Array.isArray(params.additionalDirectories) || params.additionalDirectories.length !== 0)) {
          throw new RpcError({ code: -32602, message: 'Additional workspace roots are unsupported' })
        }
        if (method === 'session/new') {
          const sessionId = await this.backend.create(cwd)
          this.sessions.add(sessionId)
          return { sessionId }
        }
        const sessionId = string(params.sessionId)
        if (this.turns.has(sessionId)) throw new RpcError({ code: -32600, message: 'Cannot load during a prompt' })
        const history = await this.backend.load(sessionId, cwd)
        for (const update of history) this.update(sessionId, update)
        this.sessions.add(sessionId)
        return {}
      }
      case 'session/prompt': {
        const sessionId = this.session(params)
        if (this.turns.has(sessionId)) throw new RpcError({ code: -32600, message: 'Steering and concurrent prompts are unsupported' })
        if (!Array.isArray(params.prompt) || params.prompt.length === 0) throw new RpcError({ code: -32602, message: 'prompt must contain content' })
        const text = params.prompt.map(value => {
          const content = object(value)
          if (content.type === 'text') return string(content.text)
          if (content.type === 'resource_link') return `${string(content.name)}: ${string(content.uri)}`
          throw new RpcError({ code: -32602, message: 'Only text and resource_link prompts are supported' })
        }).join('\n')
        const controller = new AbortController()
        this.turns.set(sessionId, controller)
        try {
          const stopReason = await this.backend.prompt(sessionId, text, update => this.update(sessionId, update), request => this.permission(sessionId, request, controller.signal), controller.signal)
          return { stopReason }
        } finally { this.turns.delete(sessionId) }
      }
      case 'session/cancel': {
        const sessionId = this.session(params)
        const turn = this.turns.get(sessionId)
        if (turn !== undefined) {
          // Only report cancellation after the daemon accepts it.
          await this.backend.cancel(sessionId)
          turn.abort()
        }
        return {}
      }
      default: throw new RpcError({ code: -32601, message: `Method not supported: ${method}` })
    }
  }

  async receive(value: unknown): Promise<void> {
    let id: string | number | null | undefined = null
    try {
      if (typeof value !== 'object' || value === null || Array.isArray(value)) {
        throw new RpcError({ code: -32600, message: 'Expected a JSON-RPC request object' })
      }
      const message = object(value)
      if (message.jsonrpc !== '2.0') throw new RpcError({ code: -32600, message: 'Expected JSON-RPC 2.0' })
      if (message.id !== undefined && typeof message.id !== 'string' && typeof message.id !== 'number') {
        throw new RpcError({ code: -32600, message: 'Request id must be a string or number' })
      }
      id = typeof message.id === 'string' || typeof message.id === 'number' ? message.id : undefined
      if (typeof message.method !== 'string') {
        const pending = typeof id === 'string' ? this.permissions.get(id) : undefined
        if (pending !== undefined) {
          this.permissions.delete(String(id))
          if (message.error !== undefined) pending.reject(new RpcError({ code: -32603, message: 'Client refused permission request' }))
          else pending.resolve(message.result)
          return
        }
        // Late replies after cancellation are responses, not new requests.
        if ('result' in message || 'error' in message) return
        throw new RpcError({ code: -32600, message: 'Expected a request or known permission response' })
      }
      const result = await this.dispatch(message.method, object(message.params ?? {}))
      if (id !== undefined) this.send({ jsonrpc: '2.0', id, result })
    } catch (cause) {
      if (id !== undefined) this.send({ jsonrpc: '2.0', id, error: { code: cause instanceof RpcError ? cause.code : -32603, message: cause instanceof Error ? cause.message : String(cause) } })
    }
  }

  /** Stdio disconnect detaches the UI, never stops or retires the daemon-owned agent. */
  close() {
    for (const pending of this.permissions.values()) pending.reject(new RpcError({ code: -32603, message: 'ACP client disconnected' }))
    this.permissions.clear()
    for (const controller of this.turns.values()) controller.abort()
  }
}
