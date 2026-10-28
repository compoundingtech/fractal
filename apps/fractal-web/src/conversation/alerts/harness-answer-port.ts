/** Positions are zero-based native option order; labels, custom text and notes never cross this port. */
export type HarnessAnswers = Readonly<Record<string, { readonly positions: readonly number[] }>>

export interface HarnessAnswerPort {
  readonly availability: 'unavailable'
  readonly answerHarnessQuestion: (request: {
    readonly callId: string
    readonly answers: HarnessAnswers
  }) => Promise<{ readonly _tag: 'Unavailable'; readonly detail: string }>
}

/** No transport is installed until metadata, fences and person-only capability admission are available. */
export const unavailableHarnessAnswers: HarnessAnswerPort = {
  availability: 'unavailable',
  answerHarnessQuestion: async () => ({
    _tag: 'Unavailable',
    detail: 'Web answers are not available yet. Answer in the terminal.',
  }),
}
