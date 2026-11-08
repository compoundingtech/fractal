import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { afterEach, beforeEach, describe, it } from 'node:test'
import { installFocusModality } from './focus-modality.ts'

class Target extends EventTarget {}

describe('installFocusModality', () => {
  const doc = new Target()
  const root = { ownerDocument: doc, dataset: {} as Record<string, string> } as unknown as HTMLElement
  const key = (init: Partial<KeyboardEvent>) => Object.assign(new Event('keydown'), init)
  const pointer = (init: Partial<PointerEvent>) => Object.assign(new Event('pointerdown'), { width: 1, height: 1, pressure: 0.5, detail: 1, ...init })
  let remove: () => void
  beforeEach(() => { remove = installFocusModality(root) })
  afterEach(() => remove())

  it('marks a real press as pointer and Tab as keyboard', () => {
    doc.dispatchEvent(pointer({}))
    assert.equal(root.dataset.inputModality, 'pointer')
    doc.dispatchEvent(key({ key: 'Tab' }))
    assert.equal(root.dataset.inputModality, 'keyboard')
  })

  it('keeps pointer through modifiers and browser chords', () => {
    doc.dispatchEvent(pointer({}))
    for (const init of [{ key: 'Meta', metaKey: true }, { key: 'Shift', shiftKey: true }, { key: 'Tab', metaKey: true }, { key: 'c', ctrlKey: true }]) doc.dispatchEvent(key(init))
    assert.equal(root.dataset.inputModality, 'pointer')
  })

  it('ignores the zero-size pointer a screen reader synthesises', () => {
    doc.dispatchEvent(key({ key: 'Tab' }))
    doc.dispatchEvent(pointer({ width: 0, height: 0 }))
    doc.dispatchEvent(pointer({ pressure: 0, detail: 0 }))
    assert.equal(root.dataset.inputModality, 'keyboard')
  })

  it('backs the tokens.css rule that hides a ring under pointer modality', () => {
    const css = readFileSync(new URL('./tokens.css', import.meta.url), 'utf8')
    assert.match(css, /:root\[data-input-modality='pointer'\] :is\(:focus-visible, \[data-focus-visible\]\) \{ outline: none !important; \}/)
  })
})
