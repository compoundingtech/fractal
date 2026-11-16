import { describe, expect, it } from 'vitest'

import { proseItem } from './semantics.ts'
import type { TextItem } from './model.ts'

const delivered = (head: string, body: string): TextItem => ({
  _tag: 'Text',
  id: 'text-1',
  role: 'user',
  text: `<smalltalk-message ${head}>\n${body}\n</smalltalk-message>`,
  attachments: [],
  streaming: false,
  at: '2026-01-01T00:00:00Z',
} as unknown as TextItem)

describe('proseItem', () => {
  it('shows a delivered body as the sender wrote it', () => {
    const item = proseItem(delivered('from="agent/demo/one"', 'Compare `a &lt; b` &amp;&amp; `Array&lt;string&gt;` "as is"'))
    expect(item.text).toBe('Compare `a < b` && `Array<string>` "as is"')
    expect(item.sender).toMatchObject({ kind: 'st-agent', label: 'demo/one' })
    expect(item.role).toBe('system')
  })

  it('unescapes once, so an authored entity stays an entity', () => {
    expect(proseItem(delivered('from="agent/a"', 'write &amp;lt; here')).text).toBe('write &lt; here')
  })

  it('reads the sender attribute through the same escaping', () => {
    expect(proseItem(delivered('from="person/a&amp;b"', 'hi')).sender).toMatchObject({ kind: 'human', label: 'a&b' })
  })

  it('turns attachment elements into a note and never into markup', () => {
    const image = proseItem(delivered('from="agent/a"', 'See <attachment path="/x/abc.png" media_type="image/png" bytes="1" name="shot &amp; one.png"/>'))
    expect(image.text).toBe('See [shot & one.png · image/png]')
    const expired = proseItem(delivered('from="agent/a"', '<attachment path="/x/abc.png" media_type="image/png" unavailable="expired"/>'))
    expect(expired.text).toBe('[attachment · expired]')
  })

  it('leaves text without an envelope alone', () => {
    const typed = { ...delivered('from="agent/a"', 'x'), text: 'a &lt; b is what I typed' } as TextItem
    expect(proseItem(typed).text).toBe('a &lt; b is what I typed')
  })
})
