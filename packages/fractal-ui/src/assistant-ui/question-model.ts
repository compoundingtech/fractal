/**
 * Kit layer: pure, typed props for the harness-neutral question form.
 * No st types, no transport, no harness names in logic (harness is display-only metadata).
 */
export type QuestionKind = 'single' | 'multi' | 'text' | 'confirm' | 'permission' | 'plan'

export type Preview =
  | { readonly _tag: 'markdown'; readonly text: string }
  | { readonly _tag: 'command'; readonly command: string; readonly cwd?: string }
  | { readonly _tag: 'diff'; readonly path: string; readonly lines: readonly string[] }
  | { readonly _tag: 'url'; readonly url: string }

export interface OptionView {
  readonly id: string
  readonly label: string
  readonly description?: string
  readonly preview?: Preview
  readonly recommended?: boolean
  /** Visual intent only: deny/decline options render with danger tone. */
  readonly tone?: 'default' | 'danger'
  /** Permission scope, rendered as a trailing chip ("this session", rule text). */
  readonly scope?: string
}

export interface QuestionView {
  readonly id: string
  /** Short chip label (Claude header <= 12 chars, omp header). Falls back to "Q1". */
  readonly header?: string
  readonly prompt: string
  readonly kind: QuestionKind
  readonly options: readonly OptionView[]
  /** Free-text "Other" row (harness adds it: omp, Claude, Codex isOther, opencode custom). */
  readonly allowOther: boolean
  /** Per-answer note (omp `n` note, Claude Tab-amend on permission). */
  readonly allowNote: boolean
  /** Question-level subject: plan markdown, command to run, diff to apply. */
  readonly subject?: Preview
  readonly placeholder?: string
  readonly secret?: boolean
  /** Default true. omp: Review may submit unanswered questions; an empty multi is a valid "none". */
  readonly required?: boolean
  /** omp note/custom answers accept images (acceptImages) sent as extra tool-result image blocks. */
  readonly allowAttachments?: boolean
}

export interface AnswerDraft {
  readonly selected: readonly string[]
  readonly other?: string
  readonly note?: string
  readonly attachments?: readonly string[]
  readonly autoSelected?: boolean
}
export type Answers = Readonly<Record<string, AnswerDraft>>

export type QuestionState =
  | { readonly _tag: 'pending'; readonly expiresAt?: string }
  | { readonly _tag: 'submitting' }
  | { readonly _tag: 'answered'; readonly answers: Answers; readonly by: string; readonly at: string; readonly where?: 'terminal' | 'web' }
  | { readonly _tag: 'answered-elsewhere'; readonly answers?: Answers; readonly where: string; readonly at: string; readonly draftDiscarded: boolean }
  | { readonly _tag: 'expired'; readonly at: string; readonly autoAnswer?: Answers }
  | { readonly _tag: 'cancelled'; readonly reason: string; readonly at: string }
  | { readonly _tag: 'failed'; readonly error: string }
  | { readonly _tag: 'chat-redirected'; readonly message: string; readonly at: string }
  /** A settled source call without recorded answer facts must not invent an outcome. */
  | { readonly _tag: 'unrecorded'; readonly reason: string }

export interface QuestionRequestView {
  readonly id: string
  /** Display-only provenance: "omp · ask", "Claude Code · Bash permission". */
  readonly source: string
  readonly agentLabel: string
  readonly askedAt: string
  readonly title?: string
  readonly questions: readonly QuestionView[]
  readonly state: QuestionState
  /** False when the harness adapter cannot deliver a web answer; the form renders read-only with a terminal hand-off. */
  readonly answerable: boolean
  readonly readOnlyReason?: string
  /** Harness offers "chat about this" (omp) / free-form response (Claude `response`). */
  readonly allowChatRedirect: boolean
  /** Decline semantics exist (MCP decline, Codex denied, omp cancel). */
  readonly declineLabel?: string
}

export type QuestionVariant = 'transcript' | 'pinned' | 'inbox'
