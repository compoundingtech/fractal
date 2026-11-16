import { it } from '@effect/vitest'
import { Schema } from 'effect'
import { describe, expect } from 'vitest'
import { isAgentTerminalPane, paneIdentity, paneKey, parsePane, subjectHref, subjectOfLocation, subjectOfRef, subjectPath, subjectRef } from './subject.ts'

const slug = (prefix = '') => Schema.String.check(Schema.isPattern(new RegExp(`^${prefix}[a-z0-9][a-z0-9 -]{0,8}(/[a-z0-9-]{1,6})?$`)))
/** The subjects a route or a ref can name: each field is what its `subjectOfRef` prefix reads back. */
const Addressable = Schema.TaggedUnion({
  Conversation: { agent: slug('agent/') },
  Terminal: { agent: slug('agent/') },
  Inbox: {},
  Alert: { id: slug() },
  Missions: {},
  Mission: { ref: slug('mission/') },
  Usage: { group: Schema.optionalKey(slug()) },
  Document: { ref: slug('doc/') },
  Declaration: { ref: slug('agent/') },
  Resource: { ref: slug('resource/github/') },
})

describe('subject addresses', () => {
  it.prop('round trips through the pane key, the route and the ref', { value: Addressable }, ({ value }) => {
    expect(parsePane(paneKey(value))).toEqual(value)
    expect(subjectOfLocation(subjectPath(value))).toEqual(value)
    expect(subjectOfRef(subjectRef(value))).toEqual(value)
  })

  it('keeps pane keys it cannot render verbatim, through the route and the ref', () => {
    for (const key of ['machine:machine/m', 'worktree:w', 'new-mission:', 'list:fleet', 'terminal:terminal/detached', 'future-kind:x y/z']) {
      const parsed = parsePane(key)
      expect(parsed._tag).toBe('Unsupported')
      expect(paneKey(parsed)).toBe(key)
      expect(subjectOfLocation(subjectPath(parsed))).toEqual(parsed)
      expect(subjectOfRef(subjectRef(parsed))).toEqual(parsed)
    }
  })

  it('gives an unknown ref a generic resource tab, on the key the TUI reads', () => {
    expect(subjectOfRef('mail/1')).toEqual({ _tag: 'Resource', ref: 'mail/1' })
    expect(paneKey(subjectOfRef('mail/1'))).toBe('resource:mail/1')
    expect(subjectOfLocation(subjectPath(subjectOfRef('mail/1')))).toEqual({ _tag: 'Resource', ref: 'mail/1' })
  })

  it('accepts links from before tabs', () => {
    expect(subjectOfLocation('/w/agent/a?open=thread')).toEqual({ _tag: 'Conversation', agent: 'agent/a' })
    expect(subjectOfLocation('/w/agent/a?open=changes')).toEqual({ _tag: 'Conversation', agent: 'agent/a' })
    expect(subjectOfLocation('/w/agent/a?open=terminal%2Fa%3Adetail')).toEqual({ _tag: 'Terminal', agent: 'agent/a' })
    expect(subjectOfLocation('/w/agent/a?open=missions%3Adetail')).toEqual({ _tag: 'Missions' })
    expect(subjectOfLocation('/w/agent/a?open=monitor%2Fquota%3Adetail')).toEqual({ _tag: 'Usage' })
    expect(subjectOfLocation('/w/agent/a?open=doc%2Fx%3Adetail')).toEqual({ _tag: 'Document', ref: 'doc/x' })
    expect(subjectOfLocation('/w/agent/a?open=mission%2Fexample%2Faudit%3Adetail')).toEqual({ _tag: 'Mission', ref: 'mission/example/audit' })
    expect(subjectOfLocation('/w/agent/a?open=alert%2Fattention%2Freview-1%3Adetail')).toEqual({ _tag: 'Alert', id: 'attention/review-1' })
  })

  it('has no subject outside the workspace route or on a malformed one', () => {
    expect(subjectOfLocation('/')).toBeUndefined()
    expect(subjectOfLocation('/w/%E0%A4%A')).toBeUndefined()
  })

  it('keeps the replay signal in the link and drops the legacy pane', () => {
    expect(subjectHref({ _tag: 'Inbox' }, 'scenario=fleet-mid-refactor&open=thread')).toBe('/w/inbox?scenario=fleet-mid-refactor')
  })

  it('gives an agent and its terminal one identity, and nothing else', () => {
    expect(paneIdentity('terminal:agent/a')).toBe('agent:agent/a')
    expect(paneIdentity('agent:agent/a')).toBe('agent:agent/a')
    expect(paneIdentity('terminal:terminal/detached')).toBe('terminal:terminal/detached')
    expect(isAgentTerminalPane('terminal:terminal/detached')).toBe(false)
  })
})
