import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import type { Undecodable } from '@st3/sdk/effect'
import { colorVars as c, typeVars as t, spaceVars as s } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { Feed, Fleet, RosterDecode } from '../data/source.ts'

/** Roster rows the latest decode could not show, never rows shown with an omitted open leaf. */
export interface HiddenRoster {
  readonly count: number
  readonly failures: readonly Undecodable[]
}

/**
 * Rows count once: a typed identity groups its field failures; an id-less row reports exactly one
 * identity (`id`) or whole-row (`$`) failure. Degraded leaves never count as hidden agents.
 */
export const hiddenRoster = ({ decode, shown }: {
  readonly decode: RosterDecode
  readonly shown: ReadonlySet<string>
}): HiddenRoster => {
  const failures = decode.undecodable.filter((entry) =>
    entry._tag === 'Rejected' && (decode.decodeStale || entry.id === undefined || !shown.has(entry.id)))
  const ids = new Set<string>()
  let anonymous = 0
  for (const entry of failures) {
    if (entry.id !== undefined) ids.add(entry.id)
    else if (entry.path === 'id' || entry.path === '$') anonymous += 1
  }
  const count = ids.size + anonymous
  return { count: count === 0 && failures.length > 0 ? 1 : count, failures }
}

export const rosterDecodeNotice = (count: number) =>
  `${count} ${count === 1 ? 'agent' : 'agents'} could not be shown · this view is older than the daemon`

/** One quiet roster notice; the disclosure carries the SDK's path and reason per failure. */
export function RosterDecodeNotice({ decode, shown }: {
  readonly decode: RosterDecode
  readonly shown: ReadonlySet<string>
}) {
  const hidden = hiddenRoster({ decode, shown })
  if (hidden.count === 0) return null
  return (
    <div data-wf-roster-decode={decode.decodeStale ? 'stale' : 'partial'} {...stylex.props(styles.notice, styles.decode)}>
      {/* Polite and atomic: only a changed count is announced, never per-row failures. */}
      <p role="status" aria-live="polite" aria-atomic="true" {...stylex.props(styles.sentence)}>{rosterDecodeNotice(hidden.count)}</p>
      <Aria.Disclosure>
        <Aria.Button slot="trigger" {...stylex.props(styles.disclosure)}>Details</Aria.Button>
        <Aria.DisclosurePanel>
          <ul tabIndex={0} aria-label="Agents that could not be shown" {...stylex.props(styles.failures)}>
            {hidden.failures.map((failure, index) => (
              <li key={`${failure.id ?? ''}:${failure.path}:${index}`} data-wf-decode-path={failure.path}>
                <span {...stylex.props(styles.identity)}>{failure.id ?? 'Row without a readable id'}</span>
                {' · '}
                <code {...stylex.props(styles.path)}>{failure.path}</code>
                {' · '}
                {failure.reason}
              </li>
            ))}
          </ul>
        </Aria.DisclosurePanel>
      </Aria.Disclosure>
    </div>
  )
}

/** The refs of the agents the roster shows, which decide which undecodable rows are still hidden. */
export const shownRosterRefs = (fleet: Feed<Fleet>): ReadonlySet<string> =>
  new Set(fleet._tag === 'Observed' ? fleet.value.agents.map((agent) => agent.ref) : [])

const styles = stylex.create({
  notice: { padding: s.lg, fontSize: t.metaSize, color: c.fgMuted },
  decode: { display: 'flex', flexDirection: 'column', gap: s.xs, paddingBlockStart: 0, flexShrink: 0 },
  sentence: { margin: 0 },
  disclosure: { alignSelf: 'flex-start', padding: 0, borderWidth: 0, backgroundColor: 'transparent', color: c.fgMuted, fontSize: t.metaSize, textDecoration: 'underline', cursor: 'pointer', outline: { default: 'none', ':focus-visible': `2px solid ${c.primary}` }, outlineOffset: 2 },
  failures: { margin: 0, paddingInlineStart: s.lg, display: 'flex', flexDirection: 'column', gap: s.xs, maxHeight: 160, overflowY: 'auto', overflowWrap: 'anywhere' },
  identity: { color: c.fg },
  path: { fontFamily: t.fontMono, fontSize: t.denseSize },
})
