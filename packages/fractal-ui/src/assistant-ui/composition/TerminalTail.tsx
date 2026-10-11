import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import type { OutputContent } from './delivery-model'
import { deliveryStyles as styles } from './delivery-styles'
import { DeliveryActionButton } from './DeliveryActionButton'
const linePageSize = 100
export function TerminalTail({ output }: { readonly output: OutputContent }) {
  const [expanded, setExpanded] = React.useState(false)
  const [page, setPage] = React.useState(0)
  const lines = React.useMemo(() => output.text.split('\n'), [output.text])
  const start = expanded ? Math.min(page * linePageSize, Math.max(0, lines.length - 1)) : Math.max(0, lines.length - 3)
  const end = expanded ? Math.min(lines.length, start + linePageSize) : lines.length
  const earlier = start + (output._tag === 'Preview' ? output.earlierLines : 0)
  const shown = lines.slice(start, end)
  return <div data-testid="terminal-tail" {...stylex.props(styles.stack)}>
    <div {...stylex.props(styles.row)}>{earlier > 0 && <span {...stylex.props(styles.muted)}>… {earlier} earlier lines</span>}{lines.length > 3 && <Button aria-expanded={expanded} onPress={() => setExpanded(value => !value)} {...stylex.props(styles.button)}>{expanded ? 'Show tail' : 'Show recorded output'}</Button>}{output._tag === 'Preview' && <><span {...stylex.props(styles.muted)}>Output preview</span><DeliveryActionButton label="Full output" action={output.fullOutput} /></>}<Button onPress={() => { void navigator.clipboard.writeText(output.text) }} {...stylex.props(styles.button)}>Copy output</Button></div>
    <pre {...stylex.props(styles.terminal)}><code>{shown.map((line, index) => <span key={start + index} data-terminal-line data-tinted={line.includes('✖') ? 'true' : 'false'} {...stylex.props(styles.line, line.includes('✖') && styles.danger)}>{line || '\u00a0'}</span>)}</code></pre>
    {expanded && <div {...stylex.props(styles.row)}><span {...stylex.props(styles.muted)}>Lines {start + 1}–{end} of {lines.length}</span>{start > 0 && <Button onPress={() => setPage(current => Math.max(0, current - 1))} {...stylex.props(styles.button)}>Previous lines</Button>}{end < lines.length && <Button onPress={() => setPage(current => current + 1)} {...stylex.props(styles.button)}>Next lines</Button>}</div>}
  </div>
}
