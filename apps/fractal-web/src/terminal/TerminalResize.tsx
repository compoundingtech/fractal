import type { TerminalScreen } from '@fractal/client/schema'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import {
  Button,
  Dialog,
  DialogTrigger,
  Input,
  Label,
  Modal,
  ModalOverlay,
  NumberField,
} from 'react-aria-components'

import type { TerminalResizePort } from './terminal-resize-port.ts'
import { borderVars, interactionVars, surfaceVars, textVars, radiusVars, spaceVars } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

/** Changing dimensions is deliberate: other viewers share the same PTY. */
export const TerminalResize = ({
  screen,
  port,
  disabledReason,
}: {
  readonly screen: TerminalScreen
  readonly port?: TerminalResizePort | undefined
  readonly disabledReason?: string | undefined
}) => {
  const [columns, setColumns] = React.useState(screen.columns)
  const [rows, setRows] = React.useState(screen.rows)
  const [status, setStatus] = React.useState<{
    readonly _tag: 'Idle' | 'Sending' | 'Result'
    readonly detail: string
  }>({ _tag: 'Idle', detail: '' })
  const unavailable =
    disabledReason ??
    (port === undefined ? 'Resize is unavailable from this data source.' : undefined)
  return (
    <DialogTrigger>
      <Button aria-label="Resize terminal" {...stylex.props(styles.button)}>
        Resize
      </Button>
      <ModalOverlay isDismissable {...stylex.props(styles.overlay)}>
        <Modal {...stylex.props(styles.modal)}>
          <Dialog aria-label="Resize shared terminal" {...stylex.props(styles.dialog)}>
            {({ close }) => (
              <>
                <h2 {...stylex.props(styles.heading)}>Resize shared terminal</h2>
                <p {...stylex.props(styles.text)}>
                  This changes the PTY for every viewer. The browser window never changes it
                  automatically.
                </p>
                <p {...stylex.props(styles.text)}>
                  Observed size: {screen.columns} columns × {screen.rows} rows.
                </p>
                <NumberField
                  value={columns}
                  onChange={setColumns}
                  minValue={1}
                  step={1}
                  isDisabled={status._tag === 'Sending'}
                  {...stylex.props(styles.field)}
                >
                  <Label>Columns</Label>
                  <Input {...stylex.props(styles.input)} />
                </NumberField>
                <NumberField
                  value={rows}
                  onChange={setRows}
                  minValue={1}
                  step={1}
                  isDisabled={status._tag === 'Sending'}
                  {...stylex.props(styles.field)}
                >
                  <Label>Rows</Label>
                  <Input {...stylex.props(styles.input)} />
                </NumberField>
                {unavailable !== undefined && (
                  <p role="status" {...stylex.props(styles.text)}>
                    {unavailable}
                  </p>
                )}
                {status.detail !== '' && (
                  <p role="status" {...stylex.props(styles.text)}>
                    {status.detail}
                  </p>
                )}
                <div {...stylex.props(styles.actions)}>
                  <Button onPress={close} {...stylex.props(styles.button)}>
                    Close
                  </Button>
                  <Button
                    isDisabled={
                      unavailable !== undefined ||
                      status._tag === 'Sending' ||
                      !Number.isSafeInteger(columns) ||
                      !Number.isSafeInteger(rows) ||
                      columns < 1 ||
                      rows < 1
                    }
                    onPress={async () => {
                      if (port === undefined) return
                      setStatus({ _tag: 'Sending', detail: 'Requesting resize…' })
                      try {
                        const result = await port.resize({
                          terminalId: screen.terminal_id,
                          incarnation: screen.runtime_incarnation,
                          columns,
                          rows,
                        })
                        setStatus({
                          _tag: 'Result',
                          detail:
                            result._tag === 'Requested'
                              ? `Resize to ${result.columns} × ${result.rows} requested. The observed terminal size confirms when it takes effect.`
                              : result.detail,
                        })
                      } catch {
                        setStatus({
                          _tag: 'Result',
                          detail:
                            'Resize could not be confirmed. Check the observed size before trying again.',
                        })
                      }
                    }}
                    {...stylex.props(styles.button)}
                  >
                    {status._tag === 'Sending' ? 'Requesting…' : 'Apply size'}
                  </Button>
                </div>
              </>
            )}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  )
}

const styles = stylex.create({
  button: {
    height: '1.5rem',
    paddingInline: spaceVars.md,
    fontSize: '0.75rem',
    color: textVars.fg,
    backgroundColor: {
      default: 'transparent',
      ':is([data-hovered])': surfaceVars.message,
    },
    borderWidth: '1px',
    borderStyle: 'solid',
    borderColor: borderVars.borderStrong,
    cursor: 'pointer',
    outlineStyle: { default: 'none', ':is([data-focus-visible])': 'solid' },
    outlineColor: interactionVars.focusRing,
    outlineWidth: '2px',
  },
  overlay: {
    position: 'fixed',
    inset: 0,
    backgroundColor: borderVars.borderStrong,
    display: 'grid',
    placeItems: 'center',
    zIndex: 100,
  },
  modal: {
    width: 'min(28rem, 90vw)',
    backgroundColor: surfaceVars.canvas,
    color: textVars.fg,
    borderRadius: radiusVars.xs,
  },
  dialog: {
    padding: spaceVars.xl,
    outline: 'none',
    display: 'flex',
    flexDirection: 'column',
    gap: spaceVars.md,
  },
  heading: { margin: 0, fontSize: '1rem' },
  text: { margin: 0, fontSize: '0.8125rem' },
  field: { display: 'flex', alignItems: 'center', gap: spaceVars.md, fontSize: '0.8125rem' },
  input: {
    width: '6rem',
    padding: spaceVars.xs,
    color: textVars.fg,
    backgroundColor: surfaceVars.canvas,
    borderWidth: '1px',
    borderStyle: 'solid',
    borderColor: borderVars.borderStrong,
  },
  actions: { display: 'flex', justifyContent: 'flex-end', gap: spaceVars.md },
})
