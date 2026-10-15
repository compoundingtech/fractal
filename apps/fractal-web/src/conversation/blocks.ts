import { sourceBlocks, type SourceBlock } from '@st3/sdk/effect'

export { SourceBlock } from '@st3/sdk/effect'

export const isRecordValue = (value: unknown): value is Readonly<Record<string, unknown>> =>
  value !== null && typeof value === 'object' && !Array.isArray(value)
export const recordValue = (value: unknown): Readonly<Record<string, unknown>> | undefined =>
  isRecordValue(value) ? value : undefined

/** Read both deployed metadata and negotiated block views without guessing from prose. */
export const blockFacts = (body: unknown): {
  readonly blocks?: readonly SourceBlock[]
  readonly view?: Readonly<Record<string, unknown>>
  readonly header?: Readonly<Record<string, unknown>>
  readonly assistantMetadata?: Readonly<Record<string, unknown>>
} => {
  const blocks = sourceBlocks(body)
  if (blocks === undefined) return {}
  const view = blocks.find(block => block.view !== undefined)?.view
  const header = blocks.find(block => block.header !== undefined)?.header
    ?? blocks.find(block => block.kind === 'header')?.view
  const assistant = blocks.find(block => block.assistant_metadata !== undefined)?.assistant_metadata
    ?? blocks.find(block => block.kind === 'assistant_metadata')?.view
    ?? blocks.find(block => (block.kind === 'text' || block.kind === 'reasoning') && block.metadata !== undefined)?.metadata
  return { blocks, ...(view === undefined ? {} : { view }), ...(header === undefined ? {} : { header }),
    ...(assistant === undefined ? {} : { assistantMetadata: assistant }) }
}
