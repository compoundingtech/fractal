import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { colorVars as c, spaceVars as s } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

/** View controls only: the workspace band under the thread header owns connection state and reconnect. */
export const SidebarChromeFooter = ({ onToggleScheme, children }: {
  readonly onToggleScheme: () => void
  readonly children?: React.ReactNode
}) => <footer {...stylex.props(styles.footer)}>
  <Aria.Button onPress={onToggleScheme} aria-label="Toggle color scheme" {...stylex.props(styles.iconButton)}><Icon name="swap" /></Aria.Button>
  {children}
</footer>

const styles = stylex.create({
  footer: { height: 40, display: 'flex', alignItems: 'center', gap: s.md, paddingInline: s.md, flexShrink: 0 },
  iconButton: { width: 28, height: 28, display: 'inline-flex', justifyContent: 'center', alignItems: 'center', backgroundColor: 'transparent', color: c.fgMuted, borderWidth: 0, borderRadius: 6, cursor: 'pointer', ':focus-visible': { outline: `2px solid ${c.primary}`, outlineOffset: 2 } },
})
