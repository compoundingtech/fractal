import assert from 'node:assert/strict'
import { readFile, readdir } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const output = resolve(process.argv[2] ?? fileURLToPath(new URL('../storybook-static/', import.meta.url)))
const preview = await readFile(resolve(output, 'iframe.html'), 'utf8')
const assets = await readdir(resolve(output, 'assets'), { recursive: true })
const stylexAssets = []
for (const asset of assets.filter(asset => asset.endsWith('.css'))) {
  const css = await readFile(resolve(output, 'assets', asset), 'utf8')
  // StyleX 0.19's default classNamePrefix is "x"; include pseudo and compound selectors.
  const classes = new Set([...css.matchAll(/\.(x[a-z0-9]+)(?=[\s{:.#,>\[])/g)].map(match => match[1]))
  if (classes.size > 0) stylexAssets.push({ path: `assets/${asset.replaceAll('\\', '/')}`, classes: classes.size })
}
assert.ok(stylexAssets.length > 0, 'The built book must contain StyleX-generated class rules')

function assertPreviewLoadsStylex(html) {
  const head = /<head\b[^>]*>([\s\S]*?)<\/head>/i.exec(html)?.[1]
  assert.ok(head, 'The built preview must have a head')
  const stylesheets = new Set()
  for (const [tag] of head.matchAll(/<link\b[^>]*>/gi)) {
    const attributes = Object.fromEntries([...tag.matchAll(/([\w-]+)\s*=\s*["']([^"']*)["']/g)].map(([, key, value]) => [key.toLowerCase(), value]))
    if (!attributes.rel?.split(/\s+/).includes('stylesheet') || !attributes.href) continue
    const url = new URL(attributes.href, 'https://storybook.invalid/iframe.html')
    if (url.origin === 'https://storybook.invalid') stylesheets.add(decodeURIComponent(url.pathname).replace(/^\//, ''))
  }
  for (const asset of stylexAssets) {
    assert.ok(stylesheets.has(asset.path), `StyleX asset ${asset.path} must load from the preview head, independently of the first story`)
  }
}

assertPreviewLoadsStylex(preview)
// Real failing control: emulate the old lazy-story-only loading without touching the output.
const withoutHeadLinks = preview.replace(/<head\b[^>]*>[\s\S]*?<\/head>/i, head => head.replace(/<link\b[^>]*>/gi, ''))
assert.throws(() => assertPreviewLoadsStylex(withoutHeadLinks), /must load from the preview head/, 'The missing-stylesheet control must be rejected')
console.log(`StyleX preview loading: ${stylexAssets.length} upfront asset(s), ${stylexAssets.reduce((sum, asset) => sum + asset.classes, 0)} generated class rules; missing-link control rejected`)
