import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { ThemePortal } from '../../../../packages/fractal-ui/src/assistant-ui/taste/ThemePortal.tsx'
import { colorVars as c, typeVars as t, spaceVars as s } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { lazyPanel, LazyPanelBoundary } from './LazyPanelBoundary.tsx'

// The panel, subject reads and redaction mapping load with the first open (or idle prefetch).
const declarationBody = lazyPanel('declaration', () => import('./DeclarationBody.tsx'))
export const prefetchDeclaration = declarationBody.prefetch

/**
 * The canonical declaration of one agent or mission in a modal sheet. Inside a DialogTrigger it
 * follows the trigger; otherwise keep it mounted and drive `isOpen`/`onOpenChange`, because the
 * portal host must exist before the sheet opens. Contents and reads exist only while open.
 * The portal stays inside the themed workspace so the sheet keeps the active colour scheme.
 */
export const DeclarationDialog = ({ subject, isOpen, onOpenChange }: {
  readonly subject: string
  readonly isOpen?: boolean
  readonly onOpenChange?: (open: boolean) => void
}) =>
  <ThemePortal>
    <Aria.ModalOverlay isDismissable {...(isOpen === undefined ? {} : { isOpen })} {...(onOpenChange === undefined ? {} : { onOpenChange })} {...stylex.props(styles.overlay)}>
      <Aria.Modal {...stylex.props(styles.modal)}>
        <LazyPanelBoundary panel={declarationBody}
          // The sheet already has its final size; only its close action shows until the body arrives.
          pending={<Aria.Dialog aria-label={subject} aria-busy="true" {...stylex.props(styles.dialog)}>{({ close }) => <Aria.Button onPress={close} {...stylex.props(styles.button)}>Close declaration</Aria.Button>}</Aria.Dialog>}
          fallback={recovery => <Aria.Dialog aria-label={subject} {...stylex.props(styles.dialog)}>{({ close }) => <>
            <p role="alert">This declaration view could not load.</p>
            <Aria.Button onPress={recovery.onPress} {...stylex.props(styles.button)}>{recovery.label}</Aria.Button>
            <Aria.Button onPress={close} {...stylex.props(styles.button)}>Close declaration</Aria.Button>
          </>}</Aria.Dialog>}>
          {module => <Aria.Dialog data-testid="declaration-dialog" {...stylex.props(styles.dialog)}>
            {({ close }) => <module.DeclarationBody subject={subject} close={<Aria.Button aria-label="Close declaration" onPress={close} {...stylex.props(styles.button)}>Close</Aria.Button>} />}
          </Aria.Dialog>}
        </LazyPanelBoundary>
      </Aria.Modal>
    </Aria.ModalOverlay>
  </ThemePortal>

const styles = stylex.create({
  overlay: { position: 'fixed', inset: 0, zIndex: 100, display: 'flex', justifyContent: 'flex-end', backgroundColor: 'rgba(0, 0, 0, 0.4)' },
  modal: { width: 736, maxWidth: 'calc(100vw - 24px)', height: '100%', backgroundColor: c.raised, color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize, outline: 'none' },
  dialog: { height: '100%', outline: 'none' },
  button: { display: 'inline-flex', alignItems: 'center', gap: s.xs, padding: '4px 8px', borderWidth: 0, borderRadius: 6, backgroundColor: 'transparent', color: c.fgMuted, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `2px solid ${c.primary}` } },
})
