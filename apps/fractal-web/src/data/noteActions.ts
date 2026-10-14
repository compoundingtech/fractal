import { ClientError, type St3Client } from '@smalltalk/st3-client'
import { SubjectNoteResult, decodeUnknownSync } from '@smalltalk/st3-client/schema'
import type { DataSource } from './source.ts'

/** Writes use the generated routes through the same-origin authenticated gateway.
 * The notes window, not the acknowledgement, publishes the changed projection. */
export const gatewayNoteActions = ({ client, actor }: {
  readonly client: Pick<St3Client, 'subjectNotesCreate' | 'subjectNotesUpdate'>
  readonly actor: () => string | undefined
}): NonNullable<DataSource['noteActions']> => {
  const submit = async (write: () => Promise<{ readonly value: unknown }>) => {
    try {
      decodeUnknownSync(SubjectNoteResult)((await write()).value)
    } catch (cause) {
      if (cause instanceof ClientError) throw new Error(`The note was not saved: ${cause.response.message}`)
      throw new Error('The note could not be confirmed. Your draft is unchanged; check your connection before trying again.')
    }
  }
  return {
    create: async (target, text, audience) => {
      const author = actor()
      if (author === undefined) throw new Error('Wait for your signed-in identity before adding a note.')
      if (author.startsWith('agent/') && audience !== 'agent') throw new Error('Agent-authored notes must be shared with the agent.')
      await submit(() => client.subjectNotesCreate({ target, text, audience }))
    },
    archive: async (note) => {
      if (note.author !== actor()) throw new Error('You can only archive your own notes.')
      await submit(() => client.subjectNotesUpdate(note.id, { text: null }))
    },
  }
}
