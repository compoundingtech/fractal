import type * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import type { StyleXStyles } from '@stylexjs/stylex'
import { colorVars as c, typeVars as t, spaceVars as s } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { lazyPanel, LazyPanelBoundary } from './LazyPanelBoundary.tsx'
import type * as Bodies from './WorkspaceMenuBodies.tsx'

/** Menu and Resources bodies share one chunk; the header triggers stay eager. */
const workspaceMenus = lazyPanel('workspaceMenus', () => import('./WorkspaceMenuBodies.tsx'))

/** Warm every workspace menu body after the roster paints and on trigger hover/focus; never spends the in-document retry. */
export const prefetchWorkspaceMenus = (): void => workspaceMenus.prefetch()

/** An opened body: warm renders at once; otherwise an empty box of `size` holds its place, with no spinner. */
export const WorkspaceMenuBody = ({ label, size, children }: {
  readonly label: string
  readonly size: StyleXStyles
  readonly children: (bodies: typeof Bodies) => React.ReactNode
}) =>
  <LazyPanelBoundary
    panel={workspaceMenus}
    pending={<div aria-busy="true" aria-label={label} {...stylex.props(size)} />}
    fallback={recovery => <div role="alert" {...stylex.props(styles.failed, size)}>
      <p {...stylex.props(styles.message)}>{label} could not load.</p>
      <Aria.Button onPress={recovery.onPress} {...stylex.props(styles.button)}>{recovery.label}</Aria.Button>
    </div>}
  >
    {children}
  </LazyPanelBoundary>

const styles = stylex.create({
  failed: { display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: s.md, padding: s.md, boxSizing: 'border-box' },
  message: { margin: 0, color: c.fg, fontSize: t.metaSize },
  button: { display: 'inline-flex', alignItems: 'center', padding: '4px 8px', borderWidth: 1, borderStyle: 'solid', borderColor: c.borderStrong, borderRadius: 6, backgroundColor: 'transparent', color: c.fg, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `2px solid ${c.primary}` } },
})
