import * as stylex from '@stylexjs/stylex'
import { colorVars as c, geometryVars as g, interactionVars, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

/** The shown conversation's transcript, else any transcript, else the shown pane's composer. */
const target = (doc: Document) =>
  doc.querySelector<HTMLElement>('[data-wf-shown-pane] [data-testid="transcript-scroll"]')
  ?? doc.querySelector<HTMLElement>('[data-testid="transcript-scroll"]')
  ?? doc.querySelector<HTMLElement>('[data-wf-shown-pane] textarea:not(:disabled)')

/**
 * The app's first tab stop: skips the devbar and sidebar to the shown conversation. Visually hidden
 * until keyboard focus reaches it; a pointer user never sees it.
 */
export function SkipLink() {
  return <a href="#conversation" {...stylex.props(styles.link)} onClick={event => {
    const next = target(event.currentTarget.ownerDocument)
    if (next === null) return
    event.preventDefault()
    if (!next.hasAttribute('tabindex') && !next.matches('a[href],button,input,select,textarea')) next.tabIndex = -1
    next.focus()
  }}>Skip to conversation</a>
}

const styles = stylex.create({
  link: {
    position: 'fixed', insetBlockStart: s.sm, insetInlineStart: s.sm, zIndex: 1000,
    width: { default: '1px', ':focus': 'auto' }, height: { default: '1px', ':focus': 'auto' },
    overflow: { default: 'hidden', ':focus': 'visible' },
    clipPath: { default: 'inset(50%)', ':focus': 'none' },
    whiteSpace: 'nowrap', paddingBlock: { default: 0, ':focus': s.xs }, paddingInline: { default: 0, ':focus': s.md },
    borderRadius: r.control, backgroundColor: c.raised, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, textDecorationLine: 'none',
    outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: g.focusRing, outlineColor: interactionVars.focusRing, outlineOffset: g.focusOffset,
  },
})
