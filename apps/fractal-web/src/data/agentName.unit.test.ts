import { describe, expect, it } from 'vitest'

import { agentDisplayName, shortAgentName } from './agentName.ts'

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
})
