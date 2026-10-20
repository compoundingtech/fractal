import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { assistantDarkTheme, compositionLightTheme } from '@smalltalk/fractal-ui/assistant-ui/shell'
import { colorVars as c, typeVars as t, spaceVars as s } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { SidebarToolbar } from '../shell/sidebar/Toolbar.tsx'
import { defaultFilters } from '../shell/sidebar/state.ts'
import { ResponsiveAgentSidebar } from './ResponsiveAgentSidebar.tsx'
import { SidebarChromeFooter } from './SidebarChromeFooter.tsx'
import { tooltipFor } from '../../../../packages/fractal-ui/src/story-play.ts'

const SidebarChromeSurface = ({ width = 256, viewport = 1440, initialScheme = 'dark' }: {
  readonly width?: number
  readonly viewport?: number
  readonly initialScheme?: 'dark' | 'light'
}) => {
  const [filters, setFilters] = React.useState(defaultFilters)
  const [scheme, setScheme] = React.useState(initialScheme)
  const [collapsed, setCollapsed] = React.useState(false)
  const [drawerOpen, setDrawerOpen] = React.useState(false)
  return <main data-sidebar-chrome-story="" style={{ width: viewport }} {...stylex.props(styles.root, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
    <ResponsiveAgentSidebar viewport={viewport} width={width} maxWidth={440} collapsed={collapsed} drawerOpen={drawerOpen} onDrawerOpenChange={setDrawerOpen} onToggleCollapsed={() => setCollapsed(value => !value)} onWidthChange={() => undefined} onWidthCommit={() => undefined} onWidthReset={() => undefined}>
      <SidebarToolbar filters={filters} update={setFilters} hosts={['local']} showRows={[{ host: 'local', asking: false, others: true }]} />
      <nav aria-label="Agent roster" {...stylex.props(styles.roster)}>Builder session</nav>
      <SidebarChromeFooter onToggleScheme={() => setScheme(value => value === 'dark' ? 'light' : 'dark')} />
    </ResponsiveAgentSidebar>
    <section {...stylex.props(styles.thread)}><Aria.Button onPress={() => { setCollapsed(false); setDrawerOpen(true) }}>Open agents</Aria.Button></section>
  </main>
}

/** Browser geometry, not just authored CSS: detects padding and content-box regressions. */
const assertChrome = (sidebar: HTMLElement): void => {
  const canvas = within(sidebar)
  const search = canvas.getByRole('textbox', { name: 'Search agents' })
  const bounds = sidebar.getBoundingClientRect()
  const field = search.getBoundingClientRect()
  if (Math.abs(field.left - bounds.left - 8) > 0.1 || Math.abs(bounds.right - field.right - 8) > 0.1) throw new Error('Search must have two 8px insets')
  if (getComputedStyle(search).borderRadius !== '6px') throw new Error('Search radius must be 6px')
  const wordmark = sidebar.querySelector('header strong')!
  if (getComputedStyle(wordmark).fontWeight !== '500') throw new Error('Wordmark weight must be 500')
  for (const button of [canvas.getByRole('button', { name: /^(Collapse|Close) agents$/ }), canvas.getByRole('button', { name: 'Toggle color scheme' })]) {
    if (Math.abs(button.getBoundingClientRect().left - bounds.left - 8) > 0.1) throw new Error('Chrome controls must align at 8px')
    const icon = button.querySelector('svg')
    if (icon?.getAttribute('stroke') !== 'currentColor' || icon.getAttribute('fill') !== 'none' || icon.getAttribute('aria-hidden') !== 'true') throw new Error('Chrome controls must use decorative stroke icons')
  }
  if (canvas.queryByRole('status') !== null) throw new Error('The sidebar chrome must not carry connection status')
}

const chromePlay = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const canvas = within(canvasElement)
  if (canvas.queryByRole('complementary', { name: 'Agents' }) === null) await userEvent.click(canvas.getByRole('button', { name: 'Open agents' }))
  const sidebar = canvas.getByRole('complementary', { name: 'Agents' })
  assertChrome(sidebar)
  const collapse = within(sidebar).getByRole('button', { name: /^(Collapse|Close) agents$/ })
  const collapseName = sidebar.dataset.sidebarPresentation === 'drawer' ? 'Close agents' : 'Collapse agents'
  await expect(collapse).toHaveAccessibleName(collapseName)
  if (sidebar.dataset.sidebarPresentation === 'drawer') await expect(collapse).toHaveFocus()
  await userEvent.hover(sidebar.querySelector('header')!)
  await userEvent.hover(collapse)
  await expect(await tooltipFor(collapse, collapseName)).toHaveTextContent(collapseName)
  await userEvent.unhover(collapse)
  const field = within(sidebar).getByRole('textbox', { name: 'Search agents' })
  const wordmark = sidebar.querySelector<HTMLElement>('header strong')!
  // Inject the reported failures into the real rendered nodes; the same criterion must reject each.
  for (const control of [
    { node: field, style: 'margin-left: -8px', error: 'Search must have two 8px insets' },
    { node: field, style: 'border-radius: 2px', error: 'Search radius must be 6px' },
    { node: wordmark, style: 'font-weight: 700', error: 'Wordmark weight must be 500' },
  ]) {
    const original = control.node.getAttribute('style')
    control.node.setAttribute('style', control.style)
    try { await expect(() => assertChrome(sidebar)).toThrow(control.error) }
    finally { if (original === null) control.node.removeAttribute('style'); else control.node.setAttribute('style', original) }
  }
  const icon = collapse.querySelector('svg')!
  const glyph = document.createTextNode('«')
  icon.replaceWith(glyph)
  try { await expect(() => assertChrome(sidebar)).toThrow('Chrome controls must use decorative stroke icons') }
  finally { glyph.replaceWith(icon) }
  await userEvent.click(within(sidebar).getByRole('button', { name: 'Toggle color scheme' }))
  assertChrome(sidebar)
  await userEvent.click(field)
  await userEvent.type(field, 'builder')
  await expect(field).toHaveValue('builder')
  if (sidebar.dataset.sidebarPresentation === 'drawer') await userEvent.clear(field)
  else await userEvent.keyboard('{Escape}')
  await expect(field).toHaveValue('')
  assertChrome(sidebar)
  await userEvent.click(collapse)
  await waitFor(() => expect(canvas.queryByRole('complementary', { name: 'Agents' })).toBeNull())
}

const meta = { title: 'wf/Sidebar/Chrome', component: SidebarChromeSurface, parameters: { layout: 'fullscreen', docs: { description: { component: 'Sidebar search uses equal 8px outer insets and a 6px radius at every dock/drawer width. Collapse and theme controls reuse decorative stroke icons without changing accessible names. The wordmark is weight 500. The chrome footer adds no status or reconnect action; connection state belongs to the workspace band under the thread header. Play rejects injected full-bleed, small-radius, bold-wordmark and Unicode-icon regressions.' } } } } satisfies Meta<typeof SidebarChromeSurface>
export default meta
type Story = StoryObj<typeof meta>
export const At256: Story = { args: { width: 256 }, play: chromePlay }
export const At320: Story = { args: { width: 320 }, play: chromePlay }
export const Light: Story = { args: { initialScheme: 'light' }, play: chromePlay }
export const NarrowDrawer: Story = { args: { width: 240, viewport: 440 }, play: chromePlay }

const styles = stylex.create({
  root: { display: 'flex', height: 480, maxWidth: '100%', overflow: 'hidden', backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  roster: { paddingInline: s.md, flexGrow: 1, minHeight: 0 },
  thread: { padding: s.md, flexGrow: 1 },
})
