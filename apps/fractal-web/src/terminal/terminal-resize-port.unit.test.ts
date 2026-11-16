import { FractalClient } from '@fractal/client'
import { describe, expect, it } from 'vitest'

import recording from '../data/subjectReadPort.gateway.fixtures.json' with { type: 'json' }
import { makeScreen } from './fixtures.ts'
import { gatewayTerminalResize } from './terminal-resize-port.ts'

const terminalId = 'terminal/agent/example'
const incarnation = 'incarnation/one'
const harness = ({ granted = true, restarted = false, status = 'accepted' } = {}) => {
  const actions: unknown[] = []
  const calls: string[] = []
  const client = new FractalClient({
    baseUrl: 'https://gateway.invalid',
    fetchImpl: async (input, init) => {
      const url = new URL(String(input))
      calls.push(url.pathname)
      if (url.pathname.endsWith('/capabilities')) return Response.json({ ...recording.capabilities, value: {
        ...recording.capabilities.value,
        capabilities: [{ id: 'terminal.resize', version: 0, state: granted ? 'granted' : 'ungranted' }],
      } })
      if (url.pathname.endsWith('/screen')) return Response.json({
        api_version: 'st3.client.v0', snapshot: recording.capabilities.snapshot,
        value: { ...makeScreen({ scene: 'F3', columns: 40, rows: 4, frame: 0 }), terminal_id: terminalId,
          runtime_incarnation: restarted ? 'incarnation/two' : incarnation, next_sequence: 11 },
      })
      actions.push(JSON.parse(String(init?.body)))
      return Response.json({ api_version: 'st3.client.v0', snapshot: recording.capabilities.snapshot, value: {
        kind: 'action-result', action_id: 'action/resize', operation_id: 'operation/resize',
        snapshot_id: recording.capabilities.snapshot.id, affected_ids: [terminalId], status,
      } })
    },
  })
  return { port: gatewayTerminalResize(client), actions, calls }
}
const request = { terminalId, incarnation, columns: 80, rows: 24 }

describe('gateway terminal resize lazy contracts', () => {
  it('decodes the observed screen and action result while preserving the fresh resize fence', async () => {
    const { port, actions } = harness()
    expect(await port.resize(request)).toEqual({ _tag: 'Requested', columns: 80, rows: 24 })
    expect(actions).toEqual([expect.objectContaining({
      type: 'terminal.resize', parameters: { terminal_id: terminalId, columns: 80, rows: 24 },
      fence: { snapshot_id: recording.capabilities.snapshot.id, subject_revisions: {}, runtime_incarnation: incarnation, terminal_sequence: 11 },
    })])
  })

  it('does not read a screen or submit when resize is ungranted', async () => {
    const { port, calls, actions } = harness({ granted: false })
    expect(await port.resize(request)).toMatchObject({ _tag: 'Refused' })
    expect(calls).toEqual(['/v1/client/capabilities'])
    expect(actions).toEqual([])
  })

  it('does not submit against a restarted incarnation', async () => {
    const { port, actions } = harness({ restarted: true })
    expect(await port.resize(request)).toMatchObject({ _tag: 'Refused', detail: expect.stringContaining('restarted') })
    expect(actions).toEqual([])
  })

  it('keeps a rejected action result a refusal', async () => {
    const { port } = harness({ status: 'rejected' })
    expect(await port.resize(request)).toMatchObject({ _tag: 'Refused', detail: expect.stringContaining('rejected') })
  })
})
