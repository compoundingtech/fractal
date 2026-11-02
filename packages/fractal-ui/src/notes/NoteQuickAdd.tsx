import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Dialog, Popover, Switch } from 'react-aria-components'
import { Button, Tooltip } from '../assistant-ui/composition/Controls'
import { ThemePortal } from '../assistant-ui/taste/ThemePortal'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as text, typeVars as t } from '../assistant-ui/composition-tokens.stylex'
import { NoteComposer, type NoteComposerProps } from './NoteComposer'
import { shortcutContextBlocked } from './shortcut-guard'

/** Mount only on the selected row. The host owns one draft per target for every entry point. */
export function NoteQuickAdd({ target, shortcutEnabled = true, ...composer }: NoteComposerProps & { readonly target: string; readonly shortcutEnabled?: boolean }) {
  const [open, setOpen] = React.useState(false)
  const [shortcutOn, setShortcutOn] = React.useState(true)
  const [announcement, setAnnouncement] = React.useState('')
  const trigger = React.useRef<HTMLButtonElement>(null)
  const enabled = shortcutEnabled && shortcutOn
  const close = () => { setOpen(false); trigger.current?.focus() }
  const shortcut = React.useCallback((node: HTMLElement | null) => {
    if (node === null || !enabled) return
    const listener = (event: KeyboardEvent) => {
      if (event.key !== 'n' || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || shortcutContextBlocked(event)) return
      event.preventDefault()
      trigger.current?.focus()
      setOpen(true)
    }
    node.ownerDocument.addEventListener('keydown', listener, true)
    return () => node.ownerDocument.removeEventListener('keydown', listener, true)
  }, [enabled])
  return <ThemePortal>
    <span ref={shortcut} data-note-quick-add {...stylex.props(styles.host)}>
      <Tooltip label={enabled ? 'Add note (N)' : 'Add note'}>
        <Button ref={trigger} variant="outline" size="sm" aria-label={`Add note to ${target}`} aria-keyshortcuts={enabled ? 'N' : undefined} onPress={() => setOpen(true)}>n</Button>
      </Tooltip>
      <span role="status" aria-live="polite" {...stylex.props(styles.announcement)}>{announcement}</span>
    </span>
    <Popover triggerRef={trigger} isOpen={open} onOpenChange={next => { if (!next) close() }} placement="bottom start" {...stylex.props(styles.popover)}>
      <Dialog aria-label={`Add note to ${target}`} {...stylex.props(styles.dialog)}>
        <NoteComposer {...composer} target={target} onClose={close} autoFocus onSubmit={async value => {
          await composer.onSubmit(value)
          setAnnouncement(`Note added to ${target}.`)
          close()
        }} />
        <Switch isSelected={shortcutOn} onChange={setShortcutOn} {...stylex.props(styles.switch)}>N shortcut {shortcutOn ? 'on' : 'off'}</Switch>
      </Dialog>
    </Popover>
  </ThemePortal>
}

const styles = stylex.create({
  host: { display: 'inline-flex', alignItems: 'center' },
  announcement: { position: 'absolute', width: 1, height: 1, overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap' },
  popover: { width: g.polishNarrow, maxWidth: 'calc(100vw - 24px)', boxSizing: 'border-box', padding: s.sm, backgroundColor: surface.raised, color: text.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md },
  dialog: { outline: 'none', minWidth: 0 },
  switch: { display: 'block', marginTop: s.sm, color: text.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':focus-visible': { outline: `2px solid ${border.focusBorder}` } },
})
