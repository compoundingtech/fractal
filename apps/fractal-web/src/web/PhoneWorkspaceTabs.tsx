import * as React from 'react'
import { mergeProps, useFocusRing, useTab, useTabList } from 'react-aria'
import { Item, useTabListState } from 'react-stately'
import type { TabListState } from 'react-stately'
import * as stylex from '@stylexjs/stylex'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex'

interface PhoneTab { readonly id: string; readonly label: string }

/** React Aria owns keyboard navigation. A shared panel keeps the thread's portal host and drafts mounted. */
export function PhoneWorkspaceTabs({ enabled = true, tabs, selected, onSelect, children }: {
  readonly enabled?: boolean
  readonly tabs: readonly PhoneTab[]
  readonly selected: string
  readonly onSelect: (id: string) => void
  readonly children: React.ReactNode
}) {
  const prefix = React.useId()
  const index = Math.max(0, tabs.findIndex(tab => tab.id === selected))
  const list = React.useRef<HTMLDivElement>(null)
  const state = useTabListState({
    items: tabs,
    children: tab => <Item key={tab.id} title={tab.label}>{tab.label}</Item>,
    selectedKey: tabs[index]!.id,
    onSelectionChange: key => onSelect(String(key)),
  })
  const { tabListProps } = useTabList({ 'aria-label': 'Workspace views' }, state, list)
  const panelId = `${prefix}-panel`
  return <div {...stylex.props(styles.root)}>
    <div {...tabListProps} ref={list} hidden={!enabled} {...stylex.props(styles.list, !enabled && styles.hidden)}>
      {tabs.map((tab, i) => <PhoneTabButton key={tab.id} tab={tab} state={state} id={`${prefix}-tab-${i}`} panelId={panelId} />)}
    </div>
    <div id={panelId} role={enabled ? 'tabpanel' : undefined} aria-labelledby={enabled ? `${prefix}-tab-${index}` : undefined} tabIndex={enabled ? 0 : undefined} {...stylex.props(styles.panel)}>{children}</div>
  </div>
}

function PhoneTabButton({ tab, state, id, panelId }: { readonly tab: PhoneTab; readonly state: TabListState<PhoneTab>; readonly id: string; readonly panelId: string }) {
  const ref = React.useRef<HTMLDivElement>(null)
  const { tabProps, isSelected } = useTab({ key: tab.id }, state, ref)
  const { focusProps, isFocusVisible } = useFocusRing()
  return <div {...mergeProps(tabProps, focusProps)} ref={ref} id={id} aria-controls={panelId} aria-label={tab.label} title={tab.label} data-selected={isSelected || undefined} data-focus-visible={isFocusVisible || undefined} {...stylex.props(styles.tab)}><span {...stylex.props(styles.label)}>{tab.label}</span></div>
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minHeight: 0, minWidth: 0 },
  hidden: { display: 'none' },
  label: { maxWidth: '14ch', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  list: { display: 'flex', flexShrink: 0, overflowX: 'auto', minHeight: 44, paddingInlineStart: `max(${s.sm}, env(safe-area-inset-left))`, paddingInlineEnd: `max(${s.sm}, env(safe-area-inset-right))`, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: c.border },
  tab: { display: 'flex', alignItems: 'center', justifyContent: 'center', minWidth: 44, minHeight: 44, paddingInline: s.lg, flexShrink: 0, boxSizing: 'border-box', fontSize: t.uiSize, color: c.fgMuted, cursor: 'pointer', borderBottomWidth: 2, borderBottomStyle: 'solid', borderBottomColor: 'transparent', ':is([data-selected])': { color: c.fg, borderBottomColor: c.primary }, ':is([data-focus-visible])': { outline: `2px solid ${c.primary}`, outlineOffset: -2 } },
  panel: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minHeight: 0, minWidth: 0, outline: 'none', ':focus-visible': { outline: `2px solid ${c.primary}`, outlineOffset: -2 } },
})
