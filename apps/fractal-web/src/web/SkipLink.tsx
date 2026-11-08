import * as stylex from '@stylexjs/stylex'
import { colorVars as c, geometryVars as g, interactionVars, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { attributeValue } from '../workspace/WorkspaceSplits.tsx'

// Only a shown conversation face carries `data-wf-shown-pane`; retained hidden tabs are inert.
const lane = '[data-wf-shown-pane] [data-testid="transcript-scroll"]'
const composer = '[data-wf-shown-pane] textarea:not(:disabled)'
const tabbable = 'a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])'

/**
 * The focused pane's conversation, else the first shown one in a split: its transcript, else its
 * composer while the transcript loads. With no conversation shown (an empty workspace, an inbox, a
 * terminal), the start of the focused pane, so the link still skips the devbar and global area.
 */
const target = (doc: Document) => {
  const frame = doc.querySelector<HTMLElement>('[data-pane][data-focused]')
  const pane = frame?.getAttribute('data-pane')
  const focused = pane == null ? [] : [...doc.querySelectorAll(`[data-surface-pane="${attributeValue(pane)}"]`)]
  for (const scope of [...focused, doc]) {
    const found = scope.querySelector<HTMLElement>(lane) ?? scope.querySelector<HTMLElement>(composer)
    if (found !== null) return found
  }
  return [...frame?.querySelectorAll<HTMLElement>(tabbable) ?? []].find(node => node.closest('[inert], [aria-hidden="true"]') === null) ?? null
}

/**
 * The app's first tab stop: skips the devbar and the global area to the focused pane's conversation. Visually hidden
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
