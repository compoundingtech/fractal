import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { syncLine } from '../../../../packages/fractal-ui/src/assistant-ui/st3-views/sync-line.ts'
import { useDataSource, useFleet, useGatewaySync, useNow } from '../data/react.tsx'
import { composerConnectionProps, type ComposerConnectionNotice } from './composerConnection.ts'

/** Surface-wide connection chrome never replaces or mounts inside retained row content. */
export const ConnectionBanner = ({ notice, retained }: { readonly notice?: ComposerConnectionNotice; readonly retained: boolean }) => notice === undefined ? null :
  <aside aria-label="Connection status" data-connection-banner={notice.tone} {...stylex.props(styles.banner)}>
    <p role="status" {...stylex.props(styles.text)}><strong>{notice.text}.</strong> {retained ? 'Last observed content stays visible and may be out of date.' : 'Waiting for live content.'} Your unsent text stays in this browser.</p>
    {notice.action === undefined ? null : <Aria.Button onPress={notice.action.onPress} {...stylex.props(styles.button)}>{notice.action.label}</Aria.Button>}
  </aside>

const online = Atom.make({ _tag: 'Online' } as const)
/** The same notice determines whether retained panes delegate transport chrome to the shell. */
export const useConnectionNotice = (): ComposerConnectionNotice | undefined => {
  const source = useDataSource(), observation = useGatewaySync(), now = useNow()
  const network = useAtomValue(source.network ?? online)
  const { connectionNotice } = composerConnectionProps({ network, observation, now, reconnect: source.reconnect })
  const failure = observation?.status._tag === 'Failed' ? syncLine({ ...observation, now, label: 'connection' }) : undefined
  const notice = connectionNotice ?? (failure === undefined ? undefined : { tone: 'reconnecting' as const, text: failure.text, ...(source.reconnect === undefined ? {} : { action: { label: 'Reconnect', onPress: source.reconnect } }) })
  return notice
}
export const LiveConnectionBanner = () => {
  const fleet = useFleet(), notice = useConnectionNotice()
  return <ConnectionBanner notice={notice} retained={fleet._tag === 'Observed'} />
}

const styles = stylex.create({
  banner: { display: 'flex', alignItems: 'center', gap: s.md, paddingBlock: s.sm, paddingInline: s.lg, flexShrink: 0, borderBottom: `1px solid ${c.border}`, backgroundColor: c.message, color: c.fgMuted, fontSize: t.metaSize },
  text: { margin: 0, flex: '1 1 0', minWidth: 0 },
  button: { backgroundColor: 'transparent', border: `1px solid ${c.borderStrong}`, borderRadius: 6, color: c.fg, padding: s.sm, cursor: 'pointer', flexShrink: 0, ':focus-visible': { outline: `2px solid ${c.primary}` } },
})
