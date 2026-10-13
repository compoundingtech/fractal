import * as stylex from '@stylexjs/stylex'
import { motionVars as m, typeVars as t } from '../assistant-ui/composition-tokens.stylex'
import { spinnerGeometry as sg } from './spinner-tokens.stylex'

export type SpinnerVariant = 'S1' | 'S2' | 'S3' | 'S4'
export type SpinnerSize = 12 | 14 | 16
export const spinnerChoices: readonly { readonly id: SpinnerVariant; readonly description: string }[] = [
  { id: 'S1', description: 'Lucide loader-circle: an open 288° arc at Lucide’s 2/24 stroke, one linear turn per second.' },
  { id: 'S2', description: 'A 270° arc over a faint full track, 1.5px stroke in currentColor, one linear turn per second.' },
  { id: 'S3', description: 'Three dots pulsing in sequence by scale; nothing rotates.' },
  { id: 'S4', description: 'TUI braille frames (⠋⠙⠹…) in monospace, stepped every 80ms.' },
]
/** The TUI's braille cycle; S4 scrolls this strip by whole frames, so only transform changes. */
export const brailleFrames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'] as const
const lucideLoaderCircle = 'M21 12a9 9 0 1 1-6.219-8.56'

/**
 * A decorative spinner in a fixed size×size layout box. Every animation targets an element inside that box and
 * changes transform only; the box itself never animates, so text beside it keeps its baseline. `reduced` forces the
 * prefers-reduced-motion frame, which the media query applies on its own as well.
 */
export function ProgressSpinner({ variant, size, reduced = false }: { readonly variant: SpinnerVariant; readonly size: SpinnerSize; readonly reduced?: boolean }) {
  const still = reduced && styles.still
  return <span aria-hidden="true" data-spinner={variant} data-spinner-size={size} data-reduced={reduced || undefined} {...stylex.props(styles.box, size === 12 ? styles.size12 : size === 14 ? styles.size14 : styles.size16)}>
    {variant === 'S1'
      ? <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" {...stylex.props(styles.glyph, styles.turn, still)}><path d={lucideLoaderCircle} /></svg>
      : variant === 'S2'
        // A size-unit viewBox keeps the stroke at exactly 1.5 CSS pixels at every size.
        ? <svg viewBox={`0 0 ${size} ${size}`} fill="none" stroke="currentColor" strokeWidth={1.5} {...stylex.props(styles.glyph, styles.turn, still)}>
          <circle cx={size / 2} cy={size / 2} r={(size - 1.5) / 2} opacity={0.25} />
          <circle cx={size / 2} cy={size / 2} r={(size - 1.5) / 2} pathLength={100} strokeDasharray="75 100" strokeLinecap="round" />
        </svg>
        : variant === 'S3'
          ? <span {...stylex.props(styles.dots)}>{[styles.lead, styles.mid, styles.tail].map((phase, index) => <svg key={index} viewBox="0 0 2 2" {...stylex.props(styles.dot, styles.pulse, phase, still)}><circle cx={1} cy={1} r={1} fill="currentColor" /></svg>)}</span>
          : <span {...stylex.props(styles.strip, styles.frames, still)}>{brailleFrames.map(frame => <span key={frame} {...stylex.props(styles.frame)}>{frame}</span>)}</span>}
  </span>
}

const rotation = stylex.keyframes({ from: { transform: 'rotate(0deg)' }, to: { transform: 'rotate(360deg)' } })
const dotPulse = stylex.keyframes({ '0%': { transform: 'scale(0.5)' }, '40%': { transform: 'scale(1)' }, '80%': { transform: 'scale(0.5)' }, '100%': { transform: 'scale(0.5)' } })
const frameStep = stylex.keyframes({ from: { transform: 'translateY(0)' }, to: { transform: 'translateY(-100%)' } })
const styles = stylex.create({
  // The layout box: fixed square, clipped, never animated. Its font size sizes the braille frames.
  box: { position: 'relative', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, overflow: 'hidden', verticalAlign: 'middle', color: 'inherit', fontFamily: t.fontMono },
  size12: { width: sg.size12, height: sg.size12, fontSize: sg.size12 },
  size14: { width: sg.size14, height: sg.size14, fontSize: sg.size14 },
  size16: { width: sg.size16, height: sg.size16, fontSize: sg.size16 },
  glyph: { display: 'block', width: '100%', height: '100%' },
  turn: { animationName: { default: rotation, '@media (prefers-reduced-motion: reduce)': 'none' }, animationDuration: m.spin, animationTimingFunction: m.linear, animationIterationCount: 'infinite' },
  dots: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', width: '100%', height: '100%' },
  dot: { display: 'block', width: sg.dot, aspectRatio: '1', flexShrink: 0 },
  pulse: { animationName: { default: dotPulse, '@media (prefers-reduced-motion: reduce)': 'none' }, animationDuration: m.spin, animationTimingFunction: m.ease, animationIterationCount: 'infinite' },
  lead: { animationDelay: sg.dotLead }, mid: { animationDelay: sg.dotMid }, tail: { animationDelay: sg.dotTail },
  strip: { position: 'absolute', insetInlineStart: 0, insetBlockStart: 0, display: 'flex', flexDirection: 'column', width: '100%' },
  frames: { animationName: { default: frameStep, '@media (prefers-reduced-motion: reduce)': 'none' }, animationDuration: sg.brailleCycle, animationTimingFunction: 'steps(10)', animationIterationCount: 'infinite' },
  frame: { display: 'block', height: '1em', lineHeight: '1em', textAlign: 'center' },
  still: { animationName: 'none' },
})
