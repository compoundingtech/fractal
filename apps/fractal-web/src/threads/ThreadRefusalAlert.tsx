import * as stylex from '@stylexjs/stylex'
import { colorVars as c, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { ThreadRefusal } from './threadSource.ts'

/**
 * A refused thread action as one alert: the sentence naming what failed, then Fractal's own reason on its own line.
 * Both lines sit inside the alert, so a screen reader announces them together.
 */
export function ThreadRefusalAlert({ refusal }: { readonly refusal: ThreadRefusal }) {
  return <div role="alert" data-thread-refusal="" {...stylex.props(styles.alert)}>
    <p {...stylex.props(styles.line, styles.title)}>{refusal.detail}</p>
    {refusal.explanation === undefined ? null : <p data-thread-refusal-reason="" {...stylex.props(styles.line, styles.reason)}>{refusal.explanation}</p>}
  </div>
}

const styles = stylex.create({
  alert: { display: 'flex', flexDirection: 'column', minWidth: 0, fontSize: t.metaSize, lineHeight: t.metaLeading },
  line: { margin: 0, overflowWrap: 'anywhere' },
  title: { color: c.dangerFg },
  reason: { color: c.fgMuted },
})
