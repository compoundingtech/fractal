import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import * as Aria from 'react-aria-components'
import { currentSpace, type Command, type WorkspaceState } from '@fractal/ui/workspace'
import { useDispatch, useWorkspace } from '@fractal/ui/workspace/react'
import { colorVars as c, interactionVars as interaction, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { namingCommand, type NamingIntent } from '../palette/dispatch.ts'

/** The space after the current one, wrapping: what Alt+G switches to. */
export const nextSpace = (state: WorkspaceState): Command | undefined => {
  if (state.spaces.length < 2) return undefined
  const index = state.spaces.findIndex(space => space.id === state.current)
  return { _tag: 'SpaceSwitch', space: state.spaces[(index + 1) % state.spaces.length]!.id }
}

const whole = (state: WorkspaceState) => state

const namingTitle: Record<NamingIntent, string> = { create: 'New space', rename: 'Rename space', duplicate: 'Duplicate space' }

type Mode = { readonly _tag: 'Menu' } | { readonly _tag: 'Naming'; readonly intent: NamingIntent } | { readonly _tag: 'Closing' } | { readonly _tag: 'Idle' }

/**
 * The spaces switcher at the top of the sidebar, in place of the wordmark: the current space's name,
 * and a menu of the person's spaces (their glasses) with new, rename, duplicate and close. Naming
 * happens in a small field; closing asks first, since it removes the space from every client.
 */
export const SpaceSwitcher = () => {
  const dispatch = useDispatch()
  const state = useWorkspace(whole)
  const { spaces, current } = state
  const name = spaces.find(space => space.id === current)?.name ?? 'Space'
  const trigger = React.useRef<HTMLButtonElement>(null)
  const [mode, setMode] = React.useState<Mode>({ _tag: 'Idle' })
  const [draft, setDraft] = React.useState('')
  const begin = (intent: NamingIntent) => {
    setDraft(intent === 'rename' ? name : intent === 'duplicate' ? `${name} copy` : '')
    setMode({ _tag: 'Naming', intent })
  }
  const submit = () => {
    if (mode._tag !== 'Naming' || draft.trim() === '') return
    dispatch(namingCommand(state, mode.intent, draft.trim()))
    setMode({ _tag: 'Idle' })
  }
  return <>
    <Aria.MenuTrigger isOpen={mode._tag === 'Menu'} onOpenChange={open => setMode(open ? { _tag: 'Menu' } : current => current._tag === 'Menu' ? { _tag: 'Idle' } : current)}>
      <Aria.Button ref={trigger} aria-label={`Space: ${name}`} data-testid="space-switcher" {...stylex.props(styles.trigger)}>
        <span {...stylex.props(styles.name)}>{name}</span>
        <Icon name="chevron-down" size={12} />
      </Aria.Button>
      <Aria.Popover placement="bottom start" {...stylex.props(styles.popover)}>
        <Aria.Menu aria-label="Spaces" autoFocus="first" onAction={key => {
          const id = String(key)
          if (id === 'new') begin('create')
          else if (id === 'rename') begin('rename')
          else if (id === 'duplicate') begin('duplicate')
          else if (id === 'close') setMode({ _tag: 'Closing' })
          else if (id.startsWith('space:')) dispatch({ _tag: 'SpaceSwitch', space: id.slice('space:'.length) })
        }} {...stylex.props(styles.menu)}>
          <Aria.MenuSection aria-label="Your spaces">
            {spaces.map(space => <Aria.MenuItem key={space.id} id={`space:${space.id}`} textValue={space.name} {...stylex.props(styles.item)}>
              <span {...stylex.props(styles.check)}>{space.id === current ? <Icon name="check" size={12} /> : null}</span>
              <span {...stylex.props(styles.name)}>{space.name}</span>
            </Aria.MenuItem>)}
          </Aria.MenuSection>
          <Aria.Separator {...stylex.props(styles.separator)} />
          <Aria.MenuSection aria-label="Change spaces">
            <Aria.MenuItem id="new" {...stylex.props(styles.item)}>New space…</Aria.MenuItem>
            <Aria.MenuItem id="rename" {...stylex.props(styles.item)}>Rename…</Aria.MenuItem>
            <Aria.MenuItem id="duplicate" {...stylex.props(styles.item)}>Duplicate…</Aria.MenuItem>
            <Aria.MenuItem id="close" isDisabled={spaces.length < 2} {...stylex.props(styles.item)}>Close space…</Aria.MenuItem>
          </Aria.MenuSection>
        </Aria.Menu>
      </Aria.Popover>
    </Aria.MenuTrigger>
    <Aria.Popover triggerRef={trigger} isOpen={mode._tag === 'Naming' || mode._tag === 'Closing'} onOpenChange={open => { if (!open) setMode({ _tag: 'Idle' }) }} placement="bottom start" {...stylex.props(styles.popover)}>
      {mode._tag === 'Naming' ? <Aria.Dialog aria-label={namingTitle[mode.intent]} data-shortcuts-paused="" {...stylex.props(styles.dialog)}>
        <form onSubmit={event => { event.preventDefault(); submit() }} {...stylex.props(styles.form)}>
          <Aria.TextField aria-label={namingTitle[mode.intent]} value={draft} onChange={setDraft} autoFocus {...stylex.props(styles.field)}>
            <Aria.Input placeholder="Space name" {...stylex.props(styles.input)} />
          </Aria.TextField>
          <Aria.Button type="submit" isDisabled={draft.trim() === ''} {...stylex.props(styles.action)}>{mode.intent === 'rename' ? 'Rename' : 'Create'}</Aria.Button>
        </form>
      </Aria.Dialog> : mode._tag === 'Closing' ? <Aria.Dialog aria-label={`Close ${name}`} data-shortcuts-paused="" {...stylex.props(styles.dialog)}>
        <p {...stylex.props(styles.prompt)}>Close “{name}”? It goes from your other clients too, the terminal included.</p>
        <div {...stylex.props(styles.form)}>
          <Aria.Button autoFocus onPress={() => setMode({ _tag: 'Idle' })} {...stylex.props(styles.action)}>Keep</Aria.Button>
          <Aria.Button onPress={() => { dispatch({ _tag: 'SpaceClose', space: currentSpace(state).id }); setMode({ _tag: 'Idle' }) }} {...stylex.props(styles.action, styles.danger)}>Close space</Aria.Button>
        </div>
      </Aria.Dialog> : null}
    </Aria.Popover>
  </>
}

const styles = stylex.create({
  trigger: {
    display: 'inline-flex', alignItems: 'center', gap: s.xs, minWidth: 0, maxWidth: '100%', paddingBlock: 2, paddingInline: s.xs,
    marginInlineStart: `calc(-1 * ${s.xs})`, borderWidth: 0, borderRadius: 6, backgroundColor: { default: 'transparent', ':hover': c.rowHover },
    color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, fontWeight: 500, cursor: 'pointer',
    outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: '2px', outlineColor: interaction.focusRing, outlineOffset: '1px',
  },
  name: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  popover: { backgroundColor: c.raised, color: c.fg, borderWidth: 1, borderStyle: 'solid', borderColor: c.border, borderRadius: 8, fontFamily: t.fontSans, fontSize: t.uiSize },
  menu: { padding: s.xs, outline: 'none', minWidth: 200, maxWidth: 320 },
  item: {
    display: 'flex', alignItems: 'center', gap: s.xs, paddingBlock: 4, paddingInline: s.sm, borderRadius: 4, cursor: 'pointer', outline: 'none',
    backgroundColor: { default: 'transparent', ':hover': c.rowHover, ':focus-visible': c.rowHover, '[data-focused]': c.rowHover }, opacity: { default: 1, '[data-disabled]': 0.5 },
  },
  check: { display: 'inline-flex', width: 12, flexShrink: 0 },
  separator: { height: 1, marginBlock: s.xs, backgroundColor: c.border },
  dialog: { padding: s.sm, outline: 'none', minWidth: 240 },
  form: { display: 'flex', gap: s.xs, alignItems: 'center' },
  field: { flexGrow: 1, minWidth: 0 },
  input: {
    width: '100%', boxSizing: 'border-box', paddingBlock: 4, paddingInline: s.sm, borderWidth: 1, borderStyle: 'solid', borderColor: c.border, borderRadius: 6,
    backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize,
    outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: '2px', outlineColor: interaction.focusRing, outlineOffset: '1px',
  },
  action: {
    paddingBlock: 4, paddingInline: s.sm, borderWidth: 1, borderStyle: 'solid', borderColor: c.border, borderRadius: 6, backgroundColor: 'transparent', color: c.fg,
    fontFamily: t.fontSans, fontSize: t.uiSize, cursor: 'pointer', opacity: { default: 1, ':disabled': 0.5 },
    outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: '2px', outlineColor: interaction.focusRing, outlineOffset: '1px',
  },
  danger: { color: c.dangerFg },
  prompt: { margin: 0, marginBlockEnd: s.sm, maxWidth: 280, color: c.fgMuted },
})
