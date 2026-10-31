import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { ChromeControl, NarrowLayoutContext } from '@fractal/ui/assistant-ui/shell'
import { spaceVars as s, geometryVars as g } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

/** View controls only: the workspace band under the thread header owns connection state and reconnect. */
export const SidebarChromeFooter = ({ onToggleScheme, children }: {
  readonly onToggleScheme: () => void
  readonly children?: React.ReactNode
}) => {
  const narrow = React.useContext(NarrowLayoutContext)
  return <footer {...stylex.props(styles.footer, narrow && styles.narrow)}>
    <ChromeControl icon="swap" label="Toggle color scheme" tooltipPlacement="top" onPress={onToggleScheme} />
    {children}
  </footer>
}

const styles = stylex.create({
  footer: { height: 40, display: 'flex', alignItems: 'center', gap: s.md, paddingInline: s.md, flexShrink: 0 },
  narrow: { minHeight: g.touchTarget },
})
