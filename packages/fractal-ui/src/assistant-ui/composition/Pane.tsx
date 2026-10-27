import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Tab, TabList, TabPanel, Tabs, ToggleButton } from 'react-aria-components'
import { Icon, type IconName } from './Icons'
import { borderVars as border, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

/**
 * The one compact pane header: 32px (`controlLg`), hairline below, leading control, identity, trailing actions.
 * Thread panes, the Changes dock and the desktop thread header share this geometry so docked headers line up.
 */
export function PaneHeader({ leading, children, actions, testId = 'pane-header', ...rest }: {
  readonly leading?: React.ReactNode
  readonly children: React.ReactNode
  readonly actions?: React.ReactNode
  readonly testId?: string
} & Omit<React.HTMLAttributes<HTMLElement>, 'className' | 'style' | 'children'>) {
  return <header data-testid={testId} {...rest} {...stylex.props(paneHeaderStyles.header)}>
    {leading}
    {children}
    {actions === undefined ? null : <span {...stylex.props(styles.actions)}>{actions}</span>}
  </header>
}

/** Header geometry for hosts that must keep their own element (e.g. the desktop thread header). */
export const paneHeaderStyles = stylex.create({
  header: { display: 'flex', alignItems: 'center', gap: s.xs, height: g.controlLg, flexShrink: 0, boxSizing: 'border-box', paddingInline: s.sm, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border, minWidth: 0 },
})

/** A pane tab: a conversation carries a glyph; a tool surface (Changes) has none and keeps its short label whole. */
export interface PaneTabItem { readonly key: string; readonly title: string; readonly glyph?: React.ReactNode }

export interface PaneProps {
  readonly tabs: readonly PaneTabItem[]
  readonly activeKey: string
  readonly onSelectTab: (key: string) => void
  /** Accessible name of the tab strip when several tabs are open. */
  readonly tabListLabel?: string
  readonly leading?: React.ReactNode
  readonly actions?: React.ReactNode
  readonly children: React.ReactNode
}

/** Same anatomy with one tab or many: a lone tab is the title, several are a tab strip; actions sit at the end. */
export function Pane({ tabs, activeKey, onSelectTab, tabListLabel = 'Open conversations', leading, actions, children }: PaneProps) {
  const active = tabs.find(tab => tab.key === activeKey) ?? tabs[0]!
  if (tabs.length === 1) return <section aria-label={active.title} data-pane={active.key} {...stylex.props(styles.pane)}>
    <PaneHeader leading={leading} actions={actions}>
      <span {...stylex.props(styles.identity)}>{active.glyph}<span title={active.title} {...stylex.props(styles.title)}>{active.title}</span></span>
    </PaneHeader>
    <div {...stylex.props(styles.body)}>{children}</div>
  </section>
  return <Tabs selectedKey={active.key} onSelectionChange={key => onSelectTab(String(key))} data-pane={active.key} {...stylex.props(styles.pane)}>
    <PaneHeader leading={leading} actions={actions}>
      <TabList aria-label={tabListLabel} {...stylex.props(styles.tabs)}>
        {tabs.map(tab => <Tab key={tab.key} id={tab.key} {...stylex.props(styles.tab, tab.glyph === undefined && styles.toolTab)}>{tab.glyph}<span title={tab.title} {...stylex.props(styles.tabLabel)}>{tab.title}</span></Tab>)}
      </TabList>
    </PaneHeader>
    {/* One panel per tab with a stable id: re-using a single panel under a changing id leaves the selected tab's aria-controls pointing at nothing. */}
    {tabs.map(tab => <TabPanel key={tab.key} id={tab.key} {...stylex.props(styles.body)}>{tab.key === active.key ? children : null}</TabPanel>)}
  </Tabs>
}

/** 24px icon action for a pane header. */
export function PaneHeaderButton({ label, icon, shortcut, onPress }: { readonly label: string; readonly icon: IconName; readonly shortcut?: string; readonly onPress: () => void }) {
  return <Button aria-label={label} aria-keyshortcuts={shortcut} onPress={onPress} {...stylex.props(styles.iconButton)}><Icon name={icon} size={14} /></Button>
}

/** 24px pressed-state icon toggle for a pane header (e.g. Show changes). */
export function PaneHeaderToggle({ label, icon, isSelected, onChange }: { readonly label: string; readonly icon: IconName; readonly isSelected: boolean; readonly onChange: (selected: boolean) => void }) {
  return <ToggleButton aria-label={label} isSelected={isSelected} onChange={onChange} {...stylex.props(styles.iconButton, styles.toggle)}><Icon name={icon} size={14} /></ToggleButton>
}

/** Wide enough that a typical agent title is readable before it truncates. */
const tabMax = '240px'
const focusRing = { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: '-2px' } as const
const styles = stylex.create({
  pane: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0 },
  identity: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0, flex: '0 1 auto', paddingInline: s.sm },
  title: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.metaSize, fontWeight: t.weightMedium, color: ink.fg },
  tabs: { display: 'flex', alignItems: 'center', gap: s.xs2, minWidth: 0, flex: '0 1 auto' },
  tab: { display: 'inline-flex', alignItems: 'center', gap: s.sm, height: g.controlSm, minWidth: 0, maxWidth: tabMax, paddingInline: s.sm, borderRadius: r.control, color: ink.fgMuted, fontSize: t.metaSize, cursor: 'pointer', outline: 'none', ':hover': { color: ink.fg }, ':is([data-selected])': { backgroundColor: surface.rowActive, color: ink.fg }, ':is([data-focus-visible])': focusRing },
  tabLabel: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  // Tool tabs (Changes) have a short fixed label; conversation titles give way first when the strip is tight.
  toolTab: { flexShrink: 0 },
  actions: { display: 'flex', alignItems: 'center', gap: s.xs2, marginInlineStart: 'auto', flexShrink: 0 },
  iconButton: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.controlSm, height: g.controlSm, flexShrink: 0, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgMuted, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, color: ink.fg }, ':focus-visible': focusRing },
  toggle: { ':is([data-selected])': { color: ink.fg, backgroundColor: surface.rowActive } },
  body: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0, outline: 'none' },
})
