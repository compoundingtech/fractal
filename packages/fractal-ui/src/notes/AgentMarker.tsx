import * as stylex from '@stylexjs/stylex'
import { geometryVars as g } from '../assistant-ui/composition-tokens.stylex'

/**
 * Agent-audience marker: a small stroke glyph that inherits `currentColor`.
 * Decorative by default; the adjacent audience label carries the meaning.
 */
export function AgentMarker({ label }: { readonly label?: string }) {
  return <svg
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    strokeWidth={1.5}
    strokeLinecap="round"
    strokeLinejoin="round"
    data-note-agent-marker=""
    aria-hidden={label === undefined ? true : undefined}
    role={label === undefined ? undefined : 'img'}
    aria-label={label}
    {...stylex.props(styles.marker)}
  >
    <path d="M8 2v2.5" />
    <rect x="2.75" y="4.5" width="10.5" height="8.5" rx="2.25" />
    <path d="M6 8.25v.5M10 8.25v.5M6.25 11h3.5" />
  </svg>
}

const styles = stylex.create({ marker: { width: g.iconSm, height: g.iconSm, flexShrink: 0 } })
