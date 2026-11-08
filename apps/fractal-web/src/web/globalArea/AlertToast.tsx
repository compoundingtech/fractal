import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { colorVars as c, elevationVars as elevation, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useAlertAnswersStore } from '../../conversation/alerts/alertAnswers.ts'

const FLASH_MS = 10_000

/**
 * The app's one toast region, at the bottom-left beside the sidebar and over the panes. It exists only while it holds
 * something, so it is never an empty landmark; each toast announces itself and none takes focus.
 */
export const ToastRegion = ({ children }: { readonly children: React.ReactNode }) =>
  <div {...stylex.props(styles.anchor)}>{children}</div>

/** A refused answer's alert has returned: said once, wherever the person has moved on to. It takes no focus and clears after 10s. */
export const AlertToast = ({ onOpen }: { readonly onOpen: (id: string) => void }): React.ReactNode => {
  const store = useAlertAnswersStore()
  const flash = store.getSnapshot().flash
  React.useEffect(() => {
    if (flash === undefined) return
    const timer = globalThis.setTimeout(store.clearFlash, FLASH_MS)
    return () => globalThis.clearTimeout(timer)
  }, [flash, store])
  if (flash === undefined) return null
  return <div role="region" aria-label="Notifications" {...stylex.props(styles.region)}>
    <div role="alert" data-testid="alert-flash" {...stylex.props(styles.toast)}>
      <p {...stylex.props(styles.text)}><strong {...stylex.props(styles.lead)}>Could not confirm your answer</strong> to “{flash.title}”. It's back; check before sending again.</p>
      <span {...stylex.props(styles.actions)}>
        <Aria.Button onPress={() => { store.clearFlash(); onOpen(flash.id) }} {...stylex.props(styles.button)}>Open</Aria.Button>
        <Aria.Button onPress={store.clearFlash} {...stylex.props(styles.button)}>Dismiss</Aria.Button>
      </span>
    </div>
  </div>
}

const styles = stylex.create({
  // Zero-height at the bottom of the workspace column, so toasts rise from its bottom-left corner and take no layout.
  anchor: { position: 'relative', height: 0, flexShrink: 0, zIndex: 15 },
  region: { position: 'absolute', insetInlineStart: s.md, insetBlockEnd: s.md, display: 'flex', flexDirection: 'column', gap: s.sm, width: `min(360px, calc(100% - 2 * ${s.md}))` },
  toast: { display: 'flex', flexDirection: 'column', gap: s.xs, paddingBlock: s.md, paddingInline: s.lg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.borderStrong, borderRadius: r.md, backgroundColor: c.raised, color: c.fg, boxShadow: elevation.popover, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  text: { margin: 0, overflowWrap: 'anywhere' },
  lead: { color: c.dangerFg, fontWeight: t.weightMedium },
  actions: { display: 'flex', gap: s.xs, marginInlineStart: `calc(-1 * ${s.xs})` },
  button: {
    minHeight: g.controlSm, paddingInline: s.xs, borderWidth: 0, borderRadius: r.sm, backgroundColor: { default: 'transparent', ':hover': c.rowHover }, color: c.fg,
    fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer',
    outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: g.focusRing, outlineColor: interaction.focusRing, outlineOffset: g.focusOffset,
  },
})
