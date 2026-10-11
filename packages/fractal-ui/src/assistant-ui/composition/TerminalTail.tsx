import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import type { OutputContent } from './delivery-model'
import { deliveryStyles as styles } from './delivery-styles'
export function TerminalTail({ output }: { readonly output: OutputContent }) {
  const [expanded, setExpanded] = React.useState(false)
  const lines = output.text.split('\n')
  const hidden = Math.max(0, lines.length - 3)
  const earlier = hidden + (output._tag === 'Preview' ? output.earlierLines : 0)
  const shown = expanded ? lines : lines.slice(-3)
  return <div data-testid="terminal-tail" {...stylex.props(styles.stack)}>
    <div {...stylex.props(styles.row)}>{earlier > 0 && <span {...stylex.props(styles.muted)}>… {earlier} earlier lines</span>}{hidden > 0 && <Button aria-expanded={expanded} onPress={() => setExpanded(value => !value)} {...stylex.props(styles.button)}>{expanded ? 'Show tail' : 'Show all recorded lines'}</Button>}{output._tag === 'Preview' && <><span {...stylex.props(styles.muted)}>Output preview</span><Button isDisabled={output.fullOutput._tag === 'Unavailable'} onPress={output.fullOutput._tag === 'Available' ? output.fullOutput.onPress : undefined} {...stylex.props(styles.button)}>Full output</Button>{output.fullOutput._tag === 'Unavailable' && <span {...stylex.props(styles.muted)}>{output.fullOutput.reason}</span>}</>}<Button onPress={() => { void navigator.clipboard.writeText(output.text) }} {...stylex.props(styles.button)}>Copy output</Button></div>
    <pre {...stylex.props(styles.terminal)}><code>{shown.map((line, index) => <span key={index} data-terminal-line data-tinted={line.includes('✖') ? 'true' : 'false'} {...stylex.props(styles.line, line.includes('✖') && styles.danger)}>{line || '\u00a0'}</span>)}</code></pre>
  </div>
}
