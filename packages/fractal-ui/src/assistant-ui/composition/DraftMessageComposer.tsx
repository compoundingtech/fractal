import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { ComposerPrimitive, useAui } from '@assistant-ui/react'
import { EmbraceComposer } from '../EmbraceComposer'
import { EmbraceRuntimeProvider } from '../EmbraceRuntime'
import type { ComposerReply } from '../embrace-composer/send'

interface Props {
  readonly label: string
  readonly draft: string
  readonly onDraftChange: (text: string) => void
  readonly onSubmit: () => void
  readonly pending?: boolean
  readonly reply?: ComposerReply
  readonly onClearReply?: () => void
  readonly inputRef?: React.Ref<HTMLTextAreaElement>
  readonly maxContentBytes?: number
}

const runtime = { messages: [], onNew: async () => {} }

/** Shared message editor for a host that owns the draft and its send receipt. */
export function DraftMessageComposer(props: Props) {
  return <EmbraceRuntimeProvider options={runtime}><ControlledDraft {...props} /></EmbraceRuntimeProvider>
}

function ControlledDraft({
  label, draft, onDraftChange, onSubmit, pending = false, reply, onClearReply, inputRef, maxContentBytes,
}: Props) {
  const aui = useAui()
  // Match the controlled native field with the kit's readiness and byte-limit state.
  React.useLayoutEffect(() => {
    if (aui.composer().getState().text !== draft) aui.composer().setText(draft)
  }, [aui, draft])
  return <EmbraceComposer
    variant="C1"
    plainText
    showQueue={false}
    conversationLabel={label}
    placeholder="Message"
    {...(reply === undefined ? {} : { reply })}
    {...(onClearReply === undefined ? {} : { onClearReply })}
    {...(maxContentBytes === undefined ? {} : { maxContentBytes })}
    sendDisabled={pending}
    onRequestSubmit={onSubmit}
    input={(style, descriptionId) => <ComposerPrimitive.Input
      ref={inputRef}
      value={draft}
      aria-label="Message"
      aria-describedby={descriptionId}
      placeholder=""
      submitMode="none"
      cancelOnEscape={false}
      readOnly={pending}
      onChange={event => onDraftChange(event.target.value)}
      {...stylex.props(style)}
    />}
  />
}
