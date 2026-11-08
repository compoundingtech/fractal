import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'
import { ChromeControl } from '../../../../packages/fractal-ui/src/assistant-ui/composition/ChromeControl.tsx'

import { colorVars as c, spaceVars as s, typeVars as t, interactionVars, geometryVars } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { AgentControl, AgentControlResult } from '../agents/agent-control-port.ts'
import { useDataSource } from '../data/react.tsx'
import type { Agent } from '../data/source.ts'

// An agent's actions, as the Fractal terminal UI's agent actions menu offers them (crates/fractal-tui/src/ui/screens.rs,
// `agent_actions`): what can be done to it now, in the same order and words. What changes the
// seat asks to confirm first; starting or resuming does not. Interrupt only stops the current turn,
// as the composer's Stop and Esc do, so it does not ask either.

export type AgentActionKey = 'interrupt' | AgentControl | 'copy'

export type AgentActionRow = {
  readonly key: AgentActionKey
  readonly label: string
  readonly what: string
  /** Changes the seat, so it asks first. */
  readonly confirm: boolean
}

/** Fractal states in which a seat has (or is getting) a process, as the Fractal terminal UI reads them. */
const RUNNING = new Set(['running', 'waiting', 'starting', 'desired'])

export const agentActions = (agent: { readonly state?: string; readonly activity?: Agent['activity'] }): readonly AgentActionRow[] => {
  const running = agent.state !== undefined && RUNNING.has(agent.state)
  return [
    ...(running && agent.activity === 'working'
      ? [{ key: 'interrupt', label: 'Interrupt', what: 'stop its current turn, as Esc does in its own screen', confirm: false } as const]
      : []),
    ...(running
      ? ([
          { key: 'restart', label: 'Restart', what: 'a new process for the same seat, on its declaration', confirm: true },
          { key: 'suspend', label: 'Suspend', what: 'stop at a quiet moment, keeping its session to resume', confirm: true },
          { key: 'retire', label: 'Retire', what: 'stop it and take it off the lists; start brings it back', confirm: true },
        ] as const)
      : ([
          { key: 'start', label: 'Start', what: 'start it again on its declaration', confirm: false },
          { key: 'resume', label: 'Resume', what: 'resume a suspended seat on the same session', confirm: false },
        ] as const)),
    { key: 'copy', label: 'Copy id', what: 'copy its id to the clipboard', confirm: false },
  ]
}

/** A group the host adds after the agent's own actions, e.g. a phone's "Open a tab" and open tabs. */
export interface OverflowSection {
  readonly title: string
  readonly items: readonly { readonly id: string; readonly label: string; readonly current?: boolean; readonly onAction: () => void }[]
}

/**
 * The header's one overflow menu: what can be done to the agent now, then View KDL, then any host sections. The agent
 * is optional because a conversation can outlive its roster row, and its declaration is still worth reading; without
 * `onViewDeclaration` (a pane that shows no agent) the menu holds only the host's sections.
 */
export const ConversationOverflowMenu = ({ agent, onViewDeclaration, sections = [] }: {
  readonly agent?: Pick<Agent, 'ref' | 'name' | 'state' | 'activity'> | undefined
  readonly onViewDeclaration?: (() => void) | undefined
  readonly sections?: readonly OverflowSection[]
}): React.ReactNode => {
  const port = useDataSource().agentControls
  const rows = agent === undefined ? [] : agentActions(agent)
  const [confirming, setConfirming] = React.useState<AgentActionRow>()
  const [status, setStatus] = React.useState<string>()
  const [busy, setBusy] = React.useState(false)
  const run = (row: AgentActionRow) => {
    setConfirming(undefined)
    if (agent === undefined) return
    if (row.key === 'copy') {
      void navigator.clipboard?.writeText(agent.ref).then(() => setStatus(`Copied ${agent.ref}`), () => setStatus(`Could not copy ${agent.ref}`))
      return
    }
    if (port === undefined) {
      setStatus('This connection cannot change agents.')
      return
    }
    setBusy(true)
    setStatus(`${row.label} ${agent.name}…`)
    const sent: Promise<AgentControlResult> = row.key === 'interrupt' ? port.interrupt(agent.ref) : port.control(agent.ref, row.key)
    void sent.then((result) => {
      setBusy(false)
      setStatus(result._tag === 'Sent' ? `${row.label} ${agent.name}: asked Fractal` : result.detail)
    })
  }
  return (
    <div {...stylex.props(styles.actions)}>
      {status ? <span role="status" {...stylex.props(styles.status)}>{status}</span> : null}
      <Aria.MenuTrigger>
        <ChromeControl icon="more" label="More actions" isDisabled={busy} />
        <Aria.Popover placement="bottom end" {...stylex.props(styles.popover)}>
          <Aria.Menu
            aria-label="More actions"
            onAction={(key) => {
              if (key === 'declaration') { onViewDeclaration?.(); return }
              const row = rows.find((each) => each.key === key)
              if (row === undefined) return
              if (row.confirm) setConfirming(row)
              else run(row)
            }}
            {...stylex.props(styles.menu)}
          >
            {/* An item's name is all of its visible words, the label then what it does, so speech input can say any of them
                (label in name). Plain spans, not label/description slots, which would name it by the label alone. */}
            {rows.map((row) => (
              <Aria.MenuItem key={row.key} id={row.key} textValue={row.label} aria-label={`${row.label}, ${row.what}`} {...stylex.props(styles.menuItem)}>
                <span {...stylex.props(styles.label)}>{row.label}</span>
                <span {...stylex.props(styles.what)}>, {row.what}</span>
              </Aria.MenuItem>
            ))}
            {onViewDeclaration === undefined ? null : <Aria.MenuItem id="declaration" textValue="View KDL" aria-label="View KDL, read its whole declaration" {...stylex.props(styles.menuItem, rows.length > 0 && styles.menuItemApart)}>
              <span {...stylex.props(styles.label)}>View KDL</span>
              <span {...stylex.props(styles.what)}>, read its whole declaration</span>
            </Aria.MenuItem>}
            {sections.map((section, index) => <Aria.MenuSection key={section.title} aria-label={section.title} {...stylex.props((index > 0 || rows.length > 0 || onViewDeclaration !== undefined) && styles.sectionApart)}>
              <Aria.Header {...stylex.props(styles.sectionTitle)}>{section.title}</Aria.Header>
              {section.items.map(item => <Aria.MenuItem key={item.id} id={`${section.title}/${item.id}`} textValue={item.label} onAction={item.onAction} {...(item.current ? { 'aria-current': 'page' as const } : {})} {...stylex.props(styles.menuItem)}>
                <span {...stylex.props(styles.label, item.current === true && styles.current)}>{item.label}</span>
              </Aria.MenuItem>)}
            </Aria.MenuSection>)}
          </Aria.Menu>
        </Aria.Popover>
      </Aria.MenuTrigger>
      <Aria.ModalOverlay isDismissable isOpen={confirming !== undefined} onOpenChange={(open) => { if (!open) setConfirming(undefined) }} {...stylex.props(styles.overlay)}>
        <Aria.Modal {...stylex.props(styles.modal)}>
          <Aria.Dialog role="alertdialog" aria-label={confirming === undefined || agent === undefined ? 'Confirm' : `${confirming.label} ${agent.name}?`} {...stylex.props(styles.dialog)}>
            {confirming === undefined || agent === undefined ? null : (
              <>
                <strong>{confirming.label} {agent.name}?</strong>
                <p {...stylex.props(styles.what)}>It will {confirming.what}.</p>
                <div {...stylex.props(styles.choices)}>
                  <Aria.Button autoFocus onPress={() => setConfirming(undefined)} {...stylex.props(styles.button, styles.bordered)}>Keep it</Aria.Button>
                  <Aria.Button onPress={() => run(confirming)} {...stylex.props(styles.button, styles.danger)}>{confirming.label}</Aria.Button>
                </div>
              </>
            )}
          </Aria.Dialog>
        </Aria.Modal>
      </Aria.ModalOverlay>
    </div>
  )
}

const styles = stylex.create({
  actions: { display: 'flex', alignItems: 'center', gap: s.md },
  status: { fontSize: t.denseSize, color: c.fgMuted, maxWidth: 280, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  button: { display: 'inline-flex', alignItems: 'center', gap: s.xs, padding: '4px 8px', borderWidth: 0, borderRadius: 6, backgroundColor: 'transparent', color: c.fgMuted, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outlineWidth: geometryVars.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing }, ':disabled': { opacity: 0.5, cursor: 'default' } },
  bordered: { borderWidth: 1, borderStyle: 'solid', borderColor: c.borderStrong, color: c.fg },
  danger: { borderWidth: 1, borderStyle: 'solid', borderColor: c.dangerFg, color: c.dangerFg },
  popover: { backgroundColor: c.raised, color: c.fg, border: `1px solid ${c.border}`, borderRadius: 6, fontFamily: t.fontSans, fontSize: t.metaSize, zIndex: 100 },
  menu: { padding: s.xs, outline: 'none', maxWidth: 360 },
  menuItem: { display: 'flex', flexDirection: 'column', gap: 2, padding: s.md, outline: 'none', borderRadius: 4, cursor: 'pointer', ':focus': { backgroundColor: c.rowHover }, ':is([data-focus-visible])': { outlineWidth: geometryVars.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing, outlineOffset: geometryVars.focusInset } },
  menuItemApart: { marginTop: s.xs, borderTopWidth: 1, borderTopStyle: 'solid', borderTopColor: c.border, borderRadius: 0 },
  sectionApart: { marginTop: s.xs, paddingTop: s.xs, borderTopWidth: 1, borderTopStyle: 'solid', borderTopColor: c.border },
  sectionTitle: { paddingInline: s.md, paddingBlock: s.xs, color: c.fgMuted, fontSize: t.denseSize },
  current: { fontWeight: 600 },
  label: { color: c.fg },
  what: { margin: 0, color: c.fgMuted, fontSize: t.denseSize },
  overlay: { position: 'fixed', inset: 0, zIndex: 100, display: 'flex', alignItems: 'center', justifyContent: 'center', backgroundColor: 'rgba(0, 0, 0, 0.4)' },
  modal: { width: 360, maxWidth: 'calc(100vw - 32px)', backgroundColor: c.raised, color: c.fg, border: `1px solid ${c.border}`, borderRadius: 8, fontFamily: t.fontSans, fontSize: t.metaSize },
  dialog: { display: 'flex', flexDirection: 'column', gap: s.md, padding: s.lg, outline: 'none' },
  choices: { display: 'flex', justifyContent: 'flex-end', gap: s.md },
})

