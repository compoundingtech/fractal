import { toolFields, toolOutput, toolString } from './embrace-tool-preview'
import type { Answers, QuestionRequestView, QuestionState, QuestionView } from './question-model'

/** The generic transcript tool seam is a terminal hand-off, never a web-answer transport. */
export type ReadOnlyToolQuestion = Omit<QuestionRequestView, 'answerable' | 'allowChatRedirect'> & { readonly answerable: false; readonly allowChatRedirect: false }

/** Normalize recorded tool args/results only. Adapters may supply the richer typed request. */
export const questionRequestFromTool = ({ name, id, input, output, status, at, agentLabel }: {
  readonly name: string
  readonly id: string
  readonly input: unknown
  readonly output: unknown
  readonly status: 'running' | 'success' | 'error' | 'interrupted'
  readonly at?: string
  readonly agentLabel?: string
}): ReadOnlyToolQuestion | undefined => {
  const fields = toolFields(input)
  const recorded = Array.isArray(fields.questions) ? fields.questions : toolString(input, ['question', 'prompt']) !== '' ? [fields] : []
  const questions: QuestionView[] = recorded.flatMap((value, index) => {
    const prompt = toolString(value, ['question', 'prompt'])
    if (prompt === '') return []
    const question = toolFields(value)
    const options = Array.isArray(question.options) ? question.options.flatMap((option, optionIndex) => {
      const label = typeof option === 'string' ? option : toolString(option, ['label', 'title'])
      if (label === '') return []
      const preview = toolString(option, ['preview'])
      return [{ id: toolString(option, ['id']) || String(optionIndex), label, description: toolString(option, ['description']) || undefined, recommended: question.recommended === optionIndex || toolFields(option).recommended === true, ...(preview !== '' ? { preview: { _tag: 'markdown' as const, text: preview } } : {}) }]
    }) : []
    return [{ id: toolString(value, ['id']) || String(index), header: toolString(value, ['header']) || undefined, prompt, kind: question.multi === true || question.multiSelect === true ? 'multi' as const : options.length === 0 ? 'text' as const : 'single' as const, options, allowOther: question.allowOther === true, allowNote: question.allowNote === true, ...(typeof question.required === 'boolean' ? { required: question.required } : {}) }]
  })
  if (questions.length === 0) return undefined
  let state: QuestionState = { _tag: 'pending' }
  if (status !== 'running') {
    const outputFields = toolFields(output)
    const details = outputFields.details === undefined ? outputFields : toolFields(outputFields.details)
    const results = Array.isArray(details.results) ? details.results : Array.isArray(details.selectedOptions) || typeof details.customInput === 'string' ? [details] : []
    const answers: Record<string, Answers[string]> = {}
    for (const [index, value] of results.entries()) {
      const result = toolFields(value)
      const resultId = toolString(value, ['id'])
      const question = resultId !== '' ? questions.find(q => q.id === resultId) : questions.find(q => q.prompt === toolString(value, ['question'])) ?? questions[index]
      if (question === undefined || (!Array.isArray(result.selectedOptions) && typeof result.customInput !== 'string')) continue
      const selected = Array.isArray(result.selectedOptions) ? result.selectedOptions.flatMap(label => typeof label === 'string' ? [question.options.find(option => option.label === label)?.id ?? label] : []) : []
      answers[question.id] = { selected, ...(typeof result.customInput === 'string' ? { other: result.customInput } : {}), ...(typeof result.note === 'string' ? { note: result.note } : {}), ...(result.timedOut === true ? { autoSelected: true } : {}) }
    }
    state = Object.keys(answers).length > 0 ? { _tag: 'answered', answers, by: toolString(details, ['by']), at: toolString(details, ['at']) || at || '' } : { _tag: 'unrecorded', reason: status === 'interrupted' ? toolOutput(output) || 'Interrupted' : status === 'error' ? toolOutput(output) : '' }
  }
  return { id, source: name, agentLabel: agentLabel ?? 'Agent', askedAt: at ?? '', questions, state, answerable: false, allowChatRedirect: false }
}
