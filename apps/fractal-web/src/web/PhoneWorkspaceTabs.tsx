import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex'

/** React Aria owns roving focus and arrow keys; one stable viewport retains the live thread. */
export function PhoneWorkspaceTabs({ tabs, selected, onSelect, children }: {
  readonly tabs: readonly { readonly id: string; readonly label: string }[]
  readonly selected: string
  readonly onSelect: (id: string) => void
  readonly children: React.ReactNode
}) {
  const index = Math.max(0, tabs.findIndex(tab => tab.id === selected))
  return <Aria.Tabs selectedKey={tabs[index]!.id} onSelectionChange={key => onSelect(String(key))} {...stylex.props(styles.root)}>
    <Aria.TabList aria-label="Workspace views" {...stylex.props(styles.list)}>
      {tabs.map(tab => <Aria.Tab key={tab.id} id={tab.id} {...stylex.props(styles.tab)}>{tab.label}</Aria.Tab>)}
    </Aria.TabList>
    <Aria.TabPanel id={tabs[index]!.id} shouldForceMount {...stylex.props(styles.panel)}>{children}</Aria.TabPanel>
  </Aria.Tabs>
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minHeight: 0, minWidth: 0 },
  list: { display: 'flex', flexShrink: 0, overflowX: 'auto', minHeight: 44, paddingInlineStart: `max(${s.sm}, env(safe-area-inset-left))`, paddingInlineEnd: `max(${s.sm}, env(safe-area-inset-right))`, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: c.border },
  tab: { display: 'flex', alignItems: 'center', justifyContent: 'center', minWidth: 44, minHeight: 44, paddingInline: s.lg, flexShrink: 0, boxSizing: 'border-box', fontSize: t.uiSize, color: c.fgMuted, cursor: 'pointer', borderBottomWidth: 2, borderBottomStyle: 'solid', borderBottomColor: 'transparent', ':is([data-selected])': { color: c.fg, borderBottomColor: c.primary }, ':is([data-focus-visible])': { outline: `2px solid ${c.primary}`, outlineOffset: -2 } },
  panel: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minHeight: 0, minWidth: 0, outline: 'none', ':focus-visible': { outline: `2px solid ${c.primary}`, outlineOffset: -2 } },
})
