import { describe, expect, it } from 'vitest'
import { sidebarAttention } from './sidebarAttention.ts'
import type { Agent } from '../data/source.ts'

const seat = (ref: string): Agent => ({ ref, ask: { _tag: 'Unknown' } } as unknown as Agent)

describe('sidebar needs-you set', () => {
  // fleet-mid-refactor: Builder and Reviewer each raised an open agent-request for the person. Alerts said 2 need you,
  // but no subject carried the attention flag, so neither row turned amber.
  it('counts an agent whose own open card asks the person, even when its subject has no attention flag', () => {
    const agents = [seat('agent/builder'), seat('agent/reviewer'), seat('agent/docs')]
    const refs = sidebarAttention({ agents, subjects: [], asking: new Set(['agent/builder', 'agent/reviewer']) })
    expect([...refs].sort()).toEqual(['agent/builder', 'agent/reviewer'])
  })
  it('never counts an asking ref outside the roster', () => {
    expect(sidebarAttention({ agents: [seat('agent/a')], subjects: [], asking: new Set(['agent/gone']) }).size).toBe(0)
  })
})
