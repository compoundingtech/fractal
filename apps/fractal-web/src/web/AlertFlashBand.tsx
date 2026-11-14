import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { colorVars as c, geometryVars as g, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useAlertAnswersStore } from '../conversation/alerts/alertAnswers.ts'

const FLASH_MS = 10_000

/** A refused answer's alert has returned: said once here, wherever the person has moved on to. It takes no focus. */
export const AlertFlashBand = ({ onOpen }: { readonly onOpen: (id: string) => void }): React.ReactNode => {
  const store = useAlertAnswersStore()
  const flash = store.getSnapshot().flash
  React.useEffect(() => {
    if (flash === undefined) return
    const timer = globalThis.setTimeout(store.clearFlash, FLASH_MS)
    return () => globalThis.clearTimeout(timer)
  }, [flash, store])
  if (flash === undefined) return null
  return <div role="alert" data-testid="alert-flash" {...stylex.props(styles.band)}>
    <span {...stylex.props(styles.text)}>Could not confirm your answer to “{flash.title}”. It's back; check before sending again.</span>
    <Aria.Button onPress={() => { store.clearFlash(); onOpen(flash.id) }} {...stylex.props(styles.button)}>Open</Aria.Button>
    <Aria.Button onPress={store.clearFlash} {...stylex.props(styles.button)}>Dismiss</Aria.Button>
  </div>
}

const styles = stylex.create({
  band: { flexShrink: 0, display: 'flex', alignItems: 'center', gap: s.md, minHeight: g.controlLg, paddingInline: s.lg, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: c.border, color: c.dangerFg, fontSize: t.metaSize },
  text: { flexGrow: 1, minWidth: 0, overflowWrap: 'anywhere' },
  button: { flexShrink: 0, minHeight: g.controlSm, borderWidth: 0, backgroundColor: 'transparent', color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { textDecoration: 'underline' } },
})
