import { describe, expect, it } from 'vitest'
import { AcpConnection, type RpcMessage } from './protocol.ts'
import type { BridgeBackend } from './backend.ts'

const fixture = () => {
  const sent: RpcMessage[] = []
  const calls: unknown[] = []
  let stopped = false
  const backend: BridgeBackend = {
    create: async cwd => { calls.push(['create', cwd]); return 'agent/acp-example' },
    load: async (id, cwd) => { calls.push(['load', id, cwd]); return [{ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'History' } }] },
    cancel: async id => { stopped = true; calls.push(['cancel', id]) },
    prompt: async (id, text, emit, permission, signal) => {
      calls.push(['prompt', id, text])
      emit({ sessionUpdate: 'tool_call', toolCallId: 'tool-1', title: 'Run a command', status: 'pending', kind: 'execute' })
      const outcome = await permission({ toolCall: { toolCallId: 'tool-1', title: 'Run a command', kind: 'execute' }, options: [{ optionId: 'allow', name: 'Allow once', kind: 'allow_once' }, { optionId: 'reject', name: 'Reject', kind: 'reject_once' }] })
      calls.push(['permission', outcome])
      if (signal.aborted) return 'cancelled'
      emit({ sessionUpdate: 'tool_call_update', toolCallId: 'tool-1', status: outcome === 'allow' ? 'completed' : 'failed' })
      emit({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Done' } })
      return 'end_turn'
    },
  }
  const connection = new AcpConnection(backend, message => sent.push(message))
  const request = (id: number, method: string, params: unknown = {}) => connection.receive({ jsonrpc: '2.0', id, method, params })
  const setup = async () => {
    await request(1, 'initialize', { protocolVersion: 2, capabilities: {} })
    await request(2, 'session/new', { cwd: '/workspace', mcpServers: [] })
  }
  return { connection, sent, calls, request, setup, stopped: () => stopped, backend }
}

describe('ACP fake client round trips', () => {
  it('negotiates v2 down to v1 without claiming unsupported capabilities', async () => {
    const f = fixture()
    await f.request(1, 'initialize', { protocolVersion: 2, capabilities: {} })
    expect(f.sent[0]).toMatchObject({ result: { protocolVersion: 1, agentCapabilities: { loadSession: true, promptCapabilities: { image: false, audio: false, embeddedContext: false } } } })
    expect(f.sent[0]?.result).not.toHaveProperty('capabilities')
    await f.request(2, 'initialize', { protocolVersion: 1, clientCapabilities: {} })
    expect(f.sent[1]).toHaveProperty('result.protocolVersion', 1)
  })

  it('streams assistant/tools and forwards an approval response exactly once', async () => {
    const f = fixture()
    await f.setup()
    const turn = f.request(3, 'session/prompt', { sessionId: 'agent/acp-example', prompt: [{ type: 'text', text: 'Hello' }] })
    const permission = f.sent.find(message => message.method === 'session/request_permission')!
    expect(permission.params).toMatchObject({ sessionId: 'agent/acp-example', toolCall: { toolCallId: 'tool-1' } })
    await f.connection.receive({ jsonrpc: '2.0', id: permission.id, result: { outcome: { outcome: 'selected', optionId: 'allow' } } })
    await turn
    expect(f.calls).toContainEqual(['permission', 'allow'])
    expect(f.sent).toContainEqual({ jsonrpc: '2.0', id: 3, result: { stopReason: 'end_turn' } })
    expect(f.sent.filter(message => message.method === 'session/update')).toHaveLength(3)
  })

  it('cancels while approval is pending and does not acknowledge the notification', async () => {
    const f = fixture()
    await f.setup()
    const turn = f.request(3, 'session/prompt', { sessionId: 'agent/acp-example', prompt: [{ type: 'text', text: 'Hello' }] })
    await f.connection.receive({ jsonrpc: '2.0', method: 'session/cancel', params: { sessionId: 'agent/acp-example' } })
    await turn
    expect(f.stopped()).toBe(true)
    expect(f.calls).toContainEqual(['permission', 'reject'])
    expect(f.sent).toContainEqual({ jsonrpc: '2.0', id: 3, result: { stopReason: 'cancelled' } })
    expect(f.sent.filter(message => 'result' in message)).toHaveLength(3)
  })

  it('reconnects and loads the daemon identity rather than creating another agent', async () => {
    const f = fixture()
    await f.setup()
    f.connection.close()
    const messages: RpcMessage[] = []
    const next = new AcpConnection(f.backend, message => messages.push(message))
    await next.receive({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } })
    await next.receive({ jsonrpc: '2.0', id: 2, method: 'session/load', params: { sessionId: 'agent/acp-example', cwd: '/workspace', mcpServers: [] } })
    expect(f.calls.filter(value => Array.isArray(value) && value[0] === 'create')).toHaveLength(1)
    expect(f.calls).toContainEqual(['load', 'agent/acp-example', '/workspace'])
    expect(messages[1]).toMatchObject({ method: 'session/update', params: { sessionId: 'agent/acp-example', update: { content: { text: 'History' } } } })
    expect(messages[2]).toEqual({ jsonrpc: '2.0', id: 2, result: {} })
  })

  it('rejects unknown approvals, concurrent steering, unsupported methods and content', async () => {
    const f = fixture()
    await f.setup()
    const turn = f.request(3, 'session/prompt', { sessionId: 'agent/acp-example', prompt: [{ type: 'text', text: 'Hello' }] })
    await f.request(4, 'session/prompt', { sessionId: 'agent/acp-example', prompt: [{ type: 'text', text: 'Steer' }] })
    expect(f.sent.at(-1)).toHaveProperty('error.code', -32600)
    const permission = f.sent.find(message => message.method === 'session/request_permission')!
    await f.connection.receive({ jsonrpc: '2.0', id: permission.id, result: { outcome: { outcome: 'selected', optionId: 'invented-approval' } } })
    await turn
    expect(f.sent.at(-1)).toHaveProperty('error.code', -32602)
    expect(f.calls).not.toContainEqual(['permission', 'allow'])
    await f.request(5, 'session/fork')
    expect(f.sent.at(-1)).toHaveProperty('error.code', -32601)
    await f.request(6, 'session/prompt', { sessionId: 'agent/acp-example', prompt: [{ type: 'image', data: 'abc' }] })
    expect(f.sent.at(-1)).toHaveProperty('error.code', -32602)
  })

  it('rejects uninitialized calls and invalid workspace/MCP parameters before creating an agent', async () => {
    const f = fixture()
    await f.request(1, 'session/new', { cwd: '/workspace', mcpServers: [] })
    expect(f.sent.at(-1)).toHaveProperty('error.code', -32600)
    await f.request(2, 'initialize', { protocolVersion: '2' })
    expect(f.sent.at(-1)).toHaveProperty('error.code', -32602)
    await f.request(3, 'initialize', { protocolVersion: 1 })
    await f.request(4, 'session/new', { cwd: 'relative', mcpServers: [] })
    await f.request(5, 'session/new', { cwd: '/workspace', mcpServers: [{ name: 'unhandled' }] })
    expect(f.calls).toEqual([])
    expect(f.sent.slice(-2)).toMatchObject([{ error: { code: -32602 } }, { error: { code: -32602 } }])
  })
})

it('returns JSON-RPC invalid-request errors and ignores late permission responses', async () => {
  const f = fixture()
  await f.connection.receive([])
  expect(f.sent[0]).toMatchObject({ id: null, error: { code: -32600 } })
  await f.connection.receive({ jsonrpc: '2.0', id: 'old-permission', result: { outcome: { outcome: 'cancelled' } } })
  expect(f.sent).toHaveLength(1)
})
