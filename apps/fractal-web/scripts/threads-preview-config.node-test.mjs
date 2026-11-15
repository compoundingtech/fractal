import assert from 'node:assert/strict'
import { test } from 'node:test'
import { derivePreviewConfig } from './threads-preview-config.mjs'

test('default preview identity remains unchanged', () => {
  assert.deepEqual(derivePreviewConfig(), {
    instance: 'threadspreview', unit: 'threadspreview-fractal-up', root: '/var/tmp/fractal-threadspreview',
  })
})

test('two preview instances have disjoint daemon units and default roots', () => {
  const web = derivePreviewConfig({ PREVIEW_INSTANCE: 'threadsweb' })
  const ios = derivePreviewConfig({ PREVIEW_INSTANCE: 'threadsios' })
  assert.deepEqual(web, { instance: 'threadsweb', unit: 'threadsweb-fractal-up', root: '/var/tmp/fractal-threadsweb' })
  assert.deepEqual(ios, { instance: 'threadsios', unit: 'threadsios-fractal-up', root: '/var/tmp/fractal-threadsios' })
  assert.notEqual(web.unit, ios.unit)
  assert.notEqual(web.root, ios.root)
})

test('explicit preview root is preserved and lowercase digits are allowed', () => {
  assert.deepEqual(derivePreviewConfig({ PREVIEW_INSTANCE: 'preview2', PREVIEW_ROOT: '/chosen/fractal-preview2' }), {
    instance: 'preview2', unit: 'preview2-fractal-up', root: '/chosen/fractal-preview2',
  })
})

test('invalid preview instance names are refused before any runtime starts', () => {
  for (const name of ['', 'Threadsweb', '../threads', 'threads web', 'threads\n', '-a', 'a-', 'a--b', 'a_b', 'a.b', 'é', '.', '..', null, 42]) {
    assert.throws(() => derivePreviewConfig({ PREVIEW_INSTANCE: name }), /PREVIEW_INSTANCE must/)
  }
})

test('kernel-valid names include single digits, internal hyphens and exactly 24 characters', () => {
  for (const name of ['a', '0', 'try-2', 'x'.repeat(24)]) {
    assert.equal(derivePreviewConfig({ PREVIEW_INSTANCE: name }).instance, name)
  }
})

test('names longer than the kernel 24-character limit are refused', () => {
  assert.throws(() => derivePreviewConfig({ PREVIEW_INSTANCE: 'x'.repeat(25) }), /1 to 24 characters/)
})
