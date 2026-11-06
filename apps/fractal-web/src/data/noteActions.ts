import { ClientError, type FractalClient } from '@fractal/client'
import { SubjectNoteResult, decodeUnknownSync } from '@fractal/client/schema'
import type { DataSource } from './source.ts'

/** Writes use the generated routes through the same-origin authenticated gateway.
 * The notes window, not the acknowledgement, publishes the changed projection. */
export const gatewayNoteActions = ({ client, actor }: {
  readonly client: Pick<FractalClient, 'subjectNotesCreate' | 'subjectNotesUpdate'>
  readonly actor: () => string | undefined
}): NonNullable<DataSource['noteActions']> => {
  const submit = async (write: () => Promise<{ readonly value: unknown }>) => {
    try {
      decodeUnknownSync(SubjectNoteResult)((await write()).value)
    } catch (cause) {
      // The composer leads with what happened to the text; these sentences say only why.
      if (cause instanceof ClientError) throw new Error(`The gateway refused this change: ${cause.response.message}`)
      throw new Error('The change could not be confirmed; the connection may have dropped. Check your connection before trying again.')
    }
  }
  return {
    create: async (target, text) => {
      if (actor() === undefined) throw new Error('Wait for your signed-in identity before adding a note.')
      await submit(() => client.subjectNotesCreate({ target, text }))
    },
    archive: async (note) => {
      if (note.author !== actor()) throw new Error('You can only archive your own notes.')
      await submit(() => client.subjectNotesUpdate(note.id, { text: null }))
    },
  }
}
