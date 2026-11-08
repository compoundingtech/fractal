import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, userEvent, within } from 'storybook/test'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import { ChromeControl } from '../../../../packages/fractal-ui/src/assistant-ui/composition/ChromeControl.tsx'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

/**
 * Three ways to end a pane's tab strip without two "···" buttons side by side. Today it ends with the strip's
 * "Open surfaces" menu, then the conversation's More actions, then the terminal and the pane's side panel toggles.
 * Each variant keeps the per-pane side panel toggle at the end and is a mock for choosing, not production wiring.
 *
 * Recommendation: B. Tab actions live where tabs live (right-click, hover ×, the "+" menu lists open tabs), so the
 * one "···" left belongs to what the tab shows, which is where people look for Stop or Restart.
 */
type Variant = 'today' | 'a' | 'b' | 'c'

const tabs = ['TerminalRenderer', 'Missions UI', 'Usage']

const MenuItems = ({ sections }: { readonly sections: readonly { readonly title?: string; readonly items: readonly string[] }[] }) =>
  <Aria.Popover placement="bottom end" {...stylex.props(styles.popover)}>
    <Aria.Menu aria-label="Menu" {...stylex.props(styles.menu)}>
      {sections.map((section, index) => <Aria.MenuSection key={index} {...stylex.props(index > 0 && styles.sectionApart)}>
        {section.title === undefined ? null : <Aria.Header {...stylex.props(styles.sectionTitle)}>{section.title}</Aria.Header>}
        {section.items.map(item => <Aria.MenuItem key={item} id={`${index}-${item}`} textValue={item} {...stylex.props(styles.item)}>{item}</Aria.MenuItem>)}
      </Aria.MenuSection>)}
    </Aria.Menu>
  </Aria.Popover>

const More = ({ label, sections }: { readonly label: string; readonly sections: Parameters<typeof MenuItems>[0]['sections'] }) =>
  <Aria.MenuTrigger><ChromeControl icon="more" label={label} /><MenuItems sections={sections} /></Aria.MenuTrigger>

const agentActions = ['Interrupt', 'Restart', 'Suspend', 'Retire', 'Copy id', 'View KDL']
const tabActions = ['Split right', 'Split down', 'Close', 'Close others']
const surfaces = ['Inbox', 'Missions', 'Usage']

const Toggles = () => <>
  <ChromeControl icon="terminal" label="Show terminal" onPress={() => undefined} />
  <ChromeControl icon="panel" label="Toggle right panel" pressed onPress={() => undefined} />
</>

const Trailing = ({ variant }: { readonly variant: Variant }) => {
  switch (variant) {
    case 'today': return <>
      <More label="Open surfaces" sections={[{ items: tabs }]} />
      <More label="More actions" sections={[{ items: agentActions }]} />
      <Toggles />
    </>
    // One menu for the pane: its tab, the pane, then the shown agent.
    case 'a': return <>
      <More label="Pane actions" sections={[{ title: 'This tab', items: tabActions }, { title: 'Open tabs', items: tabs }, { title: 'TerminalRenderer', items: agentActions }]} />
      <Toggles />
    </>
    // Tab actions on the tab itself; the strip keeps the agent's menu and the toggles.
    case 'b': return <>
      <More label="More actions" sections={[{ items: agentActions }]} />
      <Toggles />
    </>
    // No menu in the strip: the agent's actions sit at the top of its side panel, the toggles stay.
    case 'c': return <Toggles />
  }
}

const Strip = ({ variant }: { readonly variant: Variant }) =>
  <div data-strip-variant={variant} {...stylex.props(styles.strip)}>
    <div role="tablist" aria-label="Workspace tabs" {...stylex.props(styles.tabs)}>
      {tabs.map((tab, index) => <div key={tab} role="tab" aria-selected={index === 0} {...stylex.props(styles.tab, index === 0 && styles.tabActive)}>
        <span>{tab}</span>
        {variant === 'b' ? <span aria-hidden="true" data-tab-close {...stylex.props(styles.close)}><Icon name="x" size={12} /></span> : null}
      </div>)}
    </div>
    <Aria.MenuTrigger>
      <ChromeControl icon="plus" label="Open a tab" />
      <MenuItems sections={variant === 'b' || variant === 'c' ? [{ title: 'Open', items: surfaces }, { title: 'Go to tab', items: tabs }] : [{ items: surfaces }]} />
    </Aria.MenuTrigger>
    <div {...stylex.props(styles.trailing)}><Trailing variant={variant} /></div>
  </div>

const notes: Record<Variant, string> = {
  today: 'Today: two "···" side by side, one for the strip and one for the agent.',
  a: 'A: one "···" per pane, with sections for this tab, the open tabs and the shown agent. Fewest buttons, but Stop is three reads deep.',
  b: 'B (recommended): tab actions on right-click and a hover ×; the "+" menu also lists open tabs. The strip keeps the agent\'s "···", the terminal and the panel toggles.',
  c: 'C: no "···" in the strip; the agent\'s actions head its side panel. Cleanest bar, but Stop needs the panel open.',
}

const Variants = ({ scheme }: { readonly scheme: 'dark' | 'light' }) =>
  <main data-scheme={scheme} {...stylex.props(styles.root, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
    {(['today', 'a', 'b', 'c'] as const).map(variant => <figure key={variant} aria-label={notes[variant]} {...stylex.props(styles.figure)}>
      <figcaption {...stylex.props(styles.caption)}>{notes[variant]}</figcaption>
      <Strip variant={variant} />
    </figure>)}
  </main>

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/StripTrailingVariants',
  component: Variants,
  args: { scheme: 'dark' },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'A design comparison of the tab strip trailing controls, drawn from kit controls with made-up tab names.' } },
} satisfies Meta<typeof Variants>
export default meta
type Story = StoryObj<typeof meta>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['Compare:contract-fixture', 'CompareLight:contract-fixture']

export const Compare: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const more = (variant: Variant) => within(canvasElement.querySelector<HTMLElement>(`[data-strip-variant="${variant}"]`)!).queryAllByRole('button', { name: /actions|surfaces/ })
    await expect(more('today')).toHaveLength(2)
    for (const variant of ['a', 'b'] as const) await expect(more(variant)).toHaveLength(1)
    await expect(more('c')).toHaveLength(0)
    // Every variant keeps the pane's side panel toggle at the end.
    await expect(canvas.getAllByRole('button', { name: 'Toggle right panel' })).toHaveLength(4)
    await userEvent.click(within(canvasElement.querySelector<HTMLElement>('[data-strip-variant="a"]')!).getByRole('button', { name: 'Pane actions' }))
    await expect(await within(document.body).findByText('This tab')).toBeVisible()
  },
}
export const CompareLight: Story = { args: { scheme: 'light' } }

const styles = stylex.create({
  root: { minHeight: '100vh', display: 'flex', flexDirection: 'column', gap: s.xl, padding: s.xl, boxSizing: 'border-box', backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans },
  figure: { margin: 0, display: 'flex', flexDirection: 'column', gap: s.sm },
  caption: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: c.fgMuted },
  strip: { display: 'flex', alignItems: 'center', gap: s.xs, height: g.band, paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, borderRadius: r.md, maxWidth: 900 },
  tabs: { display: 'flex', alignItems: 'center', gap: s.xs2, minWidth: 0, flex: '0 1 auto', overflow: 'hidden' },
  tab: { display: 'inline-flex', alignItems: 'center', gap: s.xs, height: g.controlSm, paddingInline: s.sm, borderRadius: r.sm, color: c.fgMuted, fontSize: t.metaSize, whiteSpace: 'nowrap' },
  tabActive: { backgroundColor: c.rowActive, color: c.fg },
  close: { display: 'inline-flex', opacity: 0.6 },
  trailing: { marginInlineStart: 'auto', display: 'flex', alignItems: 'center', gap: s.xs2 },
  popover: { backgroundColor: c.raised, color: c.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, borderRadius: r.sm, fontFamily: t.fontSans, fontSize: t.metaSize },
  menu: { padding: s.xs, outline: 'none', minWidth: 180 },
  sectionApart: { marginTop: s.xs, paddingTop: s.xs, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: c.border },
  sectionTitle: { paddingInline: s.md, paddingBlock: s.xs, color: c.fgMuted, fontSize: t.denseSize },
  item: { paddingInline: s.md, paddingBlock: s.xs, borderRadius: r.sm, cursor: 'pointer', outline: 'none', ':focus': { backgroundColor: c.rowHover } },
})
