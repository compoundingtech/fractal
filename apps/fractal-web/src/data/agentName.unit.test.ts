import { describe, expect, it } from 'vitest'

import { mailOrigin } from '../conversation/mail.ts'
import { deliverySender } from '../conversation/semantics.ts'
import { agentDisplayName, isFractalMachinery, shortAgentName } from './agentName.ts'

const id = 'agent/example/atlas'

describe('agent display names', () => {
  it('shows a real roster name as declared', () => {
    expect(agentDisplayName({ id, name: 'Atlas' })).toBe('Atlas')
    expect(agentDisplayName({ id, name: 'Terminal/claude' })).toBe('Terminal/claude')
  })

  it('never shows the id, or the id without agent/, as a name', () => {
    for (const name of [undefined, '', '  ', id, 'example/atlas', ' example/atlas ']) {
      const shown = agentDisplayName({ id, name })
      expect(shown).toBe('atlas')
      expect(shown).not.toContain('/')
    }
  })

  it('shortens to the last segment', () => {
    expect(shortAgentName(id)).toBe('atlas')
    expect(shortAgentName('agent/manager/')).toBe('manager')
  })

  // Mirrors is_fractal in crates/fractal-ui-model/src/attention.rs.
  it('treats the daemon and the st3 reconciler as Fractal machinery, and nothing else', () => {
    for (const machinery of ['daemon/runtime', 'daemon/site-a', 'agent/st3/reconciler']) expect(isFractalMachinery(machinery)).toBe(true)
    for (const actor of [id, 'agent/st3/reconciler/session/1', 'agent/st3', 'agent/example/reconciler', 'person/avery', 'daemon']) expect(isFractalMachinery(actor)).toBe(false)
  })

  it('names Fractal machinery Fractal as a delivery sender and reads its mail as machine mail', () => {
    for (const from of ['daemon/runtime', 'agent/st3/reconciler']) {
      expect(deliverySender(from)).toEqual({ kind: 'system', label: 'Fractal', via: 'daemon' })
      expect(mailOrigin({ author: from, to: id })).toBe('machine')
    }
    expect(deliverySender(id)).toEqual({ kind: 'st-agent', label: 'atlas', via: 'st' })
    expect(mailOrigin({ author: id, to: 'agent/example/other' })).toBe('agent')
  })
})
