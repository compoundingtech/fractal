import * as React from 'react'
import { useAtom } from '@effect/atom-react'
import { DraftMessageComposer } from '../../../../packages/fractal-ui/src/assistant-ui/composition/DraftMessageComposer.tsx'
import type { ThreadSendResult, ThreadSource } from './threadSource.ts'

export function ThreadEditor({ source, destination, label, send, onSent, inputRef }: {
  readonly source: Pick<ThreadSource, 'draft'>
  readonly destination: string
  readonly label: string
  readonly send: (content: string, quoteId?: string) => Promise<ThreadSendResult>
  readonly onSent?: (id: string) => void
  readonly inputRef?: React.Ref<HTMLTextAreaElement>
}) {
  const [draft, setDraft] = useAtom(source.draft(destination))
  const [error, setError] = React.useState<string>()
  const [pending, setPending] = React.useState(false)
  const busy = React.useRef(false)
  const submit = async () => {
    const content = draft.content.trim()
    if (content === '' || busy.current) return
    busy.current = true
    setError(undefined)
    setPending(true)
    try {
      const result = await send(content, draft.quote?.id)
      if (result._tag === 'Refused') setError(result.detail)
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
    {pending ? <p role="status">Sending…</p> : null}
    {error === undefined ? null : <p role="alert">{error}</p>}
    <DraftMessageComposer label={label} draft={draft.content} onDraftChange={content => setDraft(current => ({ ...current, content }))}
      onSubmit={() => { void submit() }} pending={pending} maxContentBytes={8192}
      {...(inputRef === undefined ? {} : { inputRef })}
      {...(draft.quote === undefined ? {} : {
        reply: { messageId: draft.quote.id, sender: draft.quote.sender, excerpt: draft.quote.excerpt.slice(0, 160) },
        onClearReply: () => setDraft(current => ({ content: current.content })),
      })} />
  </>
}

