import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import { createServer } from 'vite'

const server = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, optimizeDeps: { noDiscovery: true, include: [] }, server: { middlewareMode: true } })
after(() => server.close())
const { glyphCollisions, glyphs, iconGroups, iconNames, iconRegistry, iconSizes, sharedGlyphAllowance } = await server.ssrLoadModule('/src/assistant-ui/composition/icon-registry.ts')
const { lucideGlyphs } = await server.ssrLoadModule('/src/assistant-ui/composition/icon-glyphs.generated.ts')

test('the registry names every meaning in the spec once, in kebab case', () => {
  assert.equal(iconNames.length, 102)
  assert.equal(new Set(iconNames).size, iconNames.length)
  for (const name of iconNames) assert.match(name, /^[a-z][a-z0-9]*(-[a-z0-9]+)*$/, name)
})

test('every entry uses a token size, a known group and a drawn glyph', () => {
  for (const [name, entry] of Object.entries(iconRegistry)) {
    assert.ok(iconSizes.includes(entry.size), `${name} size ${entry.size}`)
    assert.ok(iconGroups.includes(entry.group), `${name} group ${entry.group}`)
    assert.ok(glyphs[entry.glyph]?.length > 0, `${name} glyph ${entry.glyph}`)
    assert.ok(entry.meaning.trim().length > 0, `${name} meaning`)
  }
})

test('one meaning, one glyph: only the allowed pairs share a glyph', () => {
  assert.deepEqual(glyphCollisions(iconRegistry), [])
  // Negative control: an unlisted reuse is reported, and the allowance itself is not a blanket pass.
  const reused = { ...iconRegistry, 'synthetic-reuse': { ...iconRegistry.close } }
  assert.deepEqual(glyphCollisions(reused), [['close', 'synthetic-reuse']])
  assert.equal(glyphCollisions(iconRegistry, []).length, sharedGlyphAllowance.length)
  assert.deepEqual(glyphCollisions(iconRegistry), [])
})

test('each allowed pair really shares one glyph', () => {
  for (const [a, b] of sharedGlyphAllowance) assert.equal(iconRegistry[a].glyph, iconRegistry[b].glyph, `${a} / ${b}`)
})

test('the vendored set is exactly the set the registry draws', () => {
  const used = new Set(Object.values(iconRegistry).map(entry => entry.glyph).filter(glyph => glyph in lucideGlyphs))
  assert.deepEqual([...used].sort(), Object.keys(lucideGlyphs).sort())
})
