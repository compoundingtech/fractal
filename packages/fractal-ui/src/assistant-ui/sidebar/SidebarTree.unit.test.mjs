import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import { createServer } from 'vite'
import stylex from '@stylexjs/unplugin'

const server = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, plugins: [stylex.vite({ useCSSLayers: false })], server: { middlewareMode: true } })
const { SidebarTreeLayout } = await server.ssrLoadModule('/src/assistant-ui/sidebar/SidebarTree.tsx')
const { sidebarRowSize } = await server.ssrLoadModule('/src/assistant-ui/composition-tokens.stylex.ts')
const { Rect, Size } = await server.ssrLoadModule('react-aria-components')
after(() => server.close())

test('exact rows skip initial measurement while variable rows and touch variants retain it', () => {
  const items = [
    { id: 'agent', rowHeight: { compact: sidebarRowSize } },
    { id: 'child', rowHeight: { compact: sidebarRowSize, touch: sidebarRowSize } },
    { id: 'folder' },
  ].map(value => ({ type: 'item', key: value.id, value }))
  const collection = Object.assign(items, { size: items.length, getItem: key => items.find(item => item.key === key) })
  const layout = new SidebarTreeLayout({ estimatedRowHeight: 24 })
  layout.virtualizer = { collection, size: new Size(256, 400), visibleRect: new Rect(0, 0, 256, 400) }
  const rowContent = () => null
  layout.update({ sizeChanged: true, layoutOptions: { rowContent, touch: false } })
  assert.equal(layout.getLayoutInfo('agent').estimatedSize, false)
  assert.equal(layout.getLayoutInfo('agent').rect.height, sidebarRowSize)
  assert.equal(layout.getLayoutInfo('child').estimatedSize, false)
  assert.equal(layout.getLayoutInfo('folder').estimatedSize, true)
  layout.updateItemSize('folder', new Size(256, 61))
  assert.equal(layout.getLayoutInfo('folder').rect.height, 61)
  assert.equal(layout.getLayoutInfo('folder').estimatedSize, false)
  layout.update({ layoutOptions: { rowContent, touch: true } })
  assert.equal(layout.getLayoutInfo('agent').estimatedSize, true, 'touch mission rows are content-measured')
  assert.equal(layout.getLayoutInfo('child').estimatedSize, false)
  assert.equal(layout.getLayoutInfo('folder').estimatedSize, true, 'compact measurements cannot leak into touch layout')
})
