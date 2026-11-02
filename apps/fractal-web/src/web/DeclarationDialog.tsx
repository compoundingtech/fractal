import type { ComponentType, ReactNode } from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { useAtomValue } from '@effect/atom-react'
import { ThemePortal } from '../../../../packages/fractal-ui/src/assistant-ui/taste/ThemePortal.tsx'
import { deferredLoader, DeferredSurfaceBoundary } from '../../../../packages/fractal-ui/src/assistant-ui/composition/DeferredSurfaceBoundary.tsx'
import type { DeclarationPanelProps } from '../../../../packages/fractal-ui/src/assistant-ui/declaration/DeclarationPanel.tsx'
import { colorVars as c, typeVars as t, spaceVars as s } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useDataSource } from '../data/react.tsx'
import { agentDeclarationState, declarationSubject, missionDeclarationState } from '../declarations/model.ts'
const loadDeclarationDialog = deferredLoader<{ readonly subject: string }>(() => import('../../../../packages/fractal-ui/src/assistant-ui/declaration/DeclarationPanel.tsx').then(module => ({
  default: ({ subject }: { readonly subject: string }) => <Aria.Dialog data-testid="declaration-dialog" {...stylex.props(styles.dialog)}>
    {({ close }) => <DeclarationContents subject={subject} Panel={module.DeclarationPanel} close={<Aria.Button aria-label="Close declaration" onPress={close} {...stylex.props(styles.button)}>Close</Aria.Button>} />}
  </Aria.Dialog>,
})))
type DeclarationContentsProps = { readonly subject: string; readonly close: ReactNode; readonly Panel: ComponentType<DeclarationPanelProps> }

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
        <DeferredSurfaceBoundary load={loadDeclarationDialog} surfaceProps={{ subject }}
          pending={<Aria.Dialog aria-label={subject} {...stylex.props(styles.dialog)}>{({ close }) => <Aria.Button onPress={close} {...stylex.props(styles.button)}>Close declaration</Aria.Button>}</Aria.Dialog>}
          fallback={<Aria.Dialog aria-label={subject} {...stylex.props(styles.dialog)}>{({ close }) => <><p role="alert">This declaration view could not load.</p><Aria.Button onPress={close} {...stylex.props(styles.button)}>Close declaration</Aria.Button></>}</Aria.Dialog>} />
      </Aria.Modal>
    </Aria.ModalOverlay>
  </ThemePortal>

const DeclarationContents = ({ subject, close, Panel }: DeclarationContentsProps) => {
  const parsed = declarationSubject(subject)
  return parsed._tag === 'Agent' ? <AgentDeclaration subject={subject} close={close} Panel={Panel} />
    : parsed._tag === 'Mission' ? <MissionDeclaration subject={subject} close={close} Panel={Panel} />
    : <Panel title={subject} actions={close} state={{ _tag: 'Unavailable', message: 'Only agents and missions have a declaration to show.' }} />
}

const AgentDeclaration = ({ subject, close, Panel }: DeclarationContentsProps) => {
  const feed = useAtomValue(useDataSource().subjectReads.agentDeclaration.feed(subject))
  return <Panel title={subject} actions={close} state={agentDeclarationState(feed)} />
}

const MissionDeclaration = ({ subject, close, Panel }: DeclarationContentsProps) => {
  const feed = useAtomValue(useDataSource().subjectReads.missionDefinition.feed(subject))
  return <Panel title={subject} actions={close} state={missionDeclarationState(feed)} />
}

const styles = stylex.create({
  overlay: { position: 'fixed', inset: 0, zIndex: 100, display: 'flex', justifyContent: 'flex-end', backgroundColor: 'rgba(0, 0, 0, 0.4)' },
  modal: { width: 736, maxWidth: 'calc(100vw - 24px)', height: '100%', backgroundColor: c.raised, color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize, outline: 'none' },
  dialog: { height: '100%', outline: 'none' },
  button: { display: 'inline-flex', alignItems: 'center', gap: s.xs, padding: '4px 8px', borderWidth: 0, borderRadius: 6, backgroundColor: 'transparent', color: c.fgMuted, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `2px solid ${c.primary}` } },
})
