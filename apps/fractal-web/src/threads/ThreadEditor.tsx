import * as React from 'react'
import { useAtom } from '@effect/atom-react'
import { DraftMessageComposer } from '../../../../packages/fractal-ui/src/assistant-ui/composition/DraftMessageComposer.tsx'
import { ThreadRefusalAlert } from './ThreadRefusalAlert.tsx'
import { unreachableSend, type ThreadRefusal, type ThreadSendResult, type ThreadSource } from './threadSource.ts'

export function ThreadEditor({ source, destination, label, send, onSent, inputRef, gatewayLive }: {
  readonly source: Pick<ThreadSource, 'draft'>
  readonly destination: string
  readonly label: string
  readonly send: (content: string, quoteId?: string) => Promise<ThreadSendResult>
  readonly onSent?: (id: string) => void
  readonly inputRef?: React.Ref<HTMLTextAreaElement>
  /** Whether the gateway connection is live; `undefined` when this view does not observe it. */
  readonly gatewayLive?: boolean | undefined
}) {
  const [draft, setDraft] = useAtom(source.draft(destination))
  const [error, setError] = React.useState<{ readonly refusal: ThreadRefusal; readonly whileDown: boolean }>()
  const [pending, setPending] = React.useState(false)
  const busy = React.useRef(false)
  // A send that could not reach Fractal while it was down says to try again once it reconnects; once it has, that advice is spent.
  const shownError = error === undefined || (error.whileDown && gatewayLive === true) ? undefined : error.refusal
  const submit = async () => {
    const content = draft.content.trim()
    if (content === '' || busy.current) return
    busy.current = true
    setError(undefined)
    setPending(true)
    try {
      const result = await send(content, draft.quote?.id)
      if (result._tag === 'Refused') setError({ refusal: result, whileDown: result.detail === unreachableSend && gatewayLive === false })
      else {
        setDraft(current => current === draft ? { content: '' } : current)
        onSent?.(result.threadId)
      }
    } finally {
      busy.current = false
      setPending(false)
    }
  }
  return <>
    {shownError === undefined ? null : <ThreadRefusalAlert refusal={shownError} />}
    <DraftMessageComposer label={label} draft={draft.content} onDraftChange={content => setDraft(current => ({ ...current, content }))}
      onSubmit={() => { void submit() }} pending={pending} maxContentBytes={8192}
      {...(inputRef === undefined ? {} : { inputRef })}
      {...(draft.quote === undefined ? {} : {
        reply: { messageId: draft.quote.id, sender: draft.quote.sender, excerpt: draft.quote.excerpt.slice(0, 160) },
        onClearReply: () => setDraft(current => ({ content: current.content })),
      })} />
  </>
}

