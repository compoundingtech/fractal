import type * as Native from '@fractal/client/schema'
import { Option } from 'effect'

import { alertsIn, alertConversations } from '../../../../../clients/typescript/fractal-views/homeView.ts'
import { ANSWERS, isRequest, personAnswer, yesNo } from '../../../../../clients/typescript/fractal-views/requestView.ts'
import type { AlertAction } from './alert-action-port.ts'

// What an alert offers, separate from how it looks: the answers the Fractal terminal UI and the phone give (a structured
// request's named answers, Yes and No for a yes-or-no question, the person's own words, Nothing to do; a gate's approve,
// send back or reject; a launch's approve, send back to its planner, or cancel), and which of them a card shows as options.

/** One answer an alert offers, and what it sends. */
export type AlertChoice = {
  readonly key: string
  readonly label: string
  /** What happens next, as the request says. */
  readonly consequence?: string
  readonly recommended?: boolean
  /** Whether this answer takes the person's words: never, if they wrote some, or only with them. */
  readonly words: 'none' | 'optional' | 'required'
  readonly tone: 'primary' | 'secondary' | 'danger'
  readonly action: (words: string) => AlertAction
}

const done = (summary: string, answer?: { readonly id?: string; readonly text?: string }): AlertAction => ({
  type: 'work.done',
  summary,
  ...(answer === undefined ? {} : { answer }),
})

/** The person's answer to a structured request, as the Fractal terminal UI types it; the words become the summary. */
const typed = (request: NonNullable<Native.Attention['request']>, chosen: string | undefined, words: string) => {
  const answer = personAnswer(request as Parameters<typeof personAnswer>[0], chosen, words)
  return typeof answer === 'object' ? answer : undefined
}

/**
 * The answers an alert offers here, or null when it is answered elsewhere. A request is answered
 * only while Fractal offers `work.done` on it; a gate offers only the `review.*` actions Fractal lists.
 */
export const alertChoices = (alert: Native.Attention): readonly AlertChoice[] | null => {
  const offered = alert.actions as readonly string[]
  if (isRequest(alert.attention_kind) && offered.includes('work.done')) {
    const request = alert.request
    if (request !== undefined) {
      const choices: AlertChoice[] = (request.answers ?? []).map((answer) => ({
        key: answer.id,
        label: answer.label,
        consequence: answer.consequence,
        recommended: request.recommendation?.answer === answer.id,
        // Requesting changes carries the changes, in the person's words.
        words: answer.outcome === 'request_changes' ? 'required' : 'none',
        tone: answer.outcome === 'decline' ? 'danger' : request.recommendation?.answer === answer.id ? 'primary' : 'secondary',
        action: (words) => done(answer.outcome === 'request_changes' ? words.trim() : answer.label, typed(request, answer.id, words)),
      }))
      if (request.type === 'feedback' || (request.type === 'choice' && request.custom === true)) {
        choices.push({
          key: 'words', label: 'Send', words: 'required', tone: choices.length === 0 ? 'primary' : 'secondary',
          action: (words) => done(words.trim(), typed(request, undefined, words)),
        })
      }
      // A request with named answers is set aside by name; Fractal tells its asker it was dismissed.
      choices.push((request.answers ?? []).length > 0
        ? { key: 'dismiss', label: 'Dismiss', words: 'none', tone: 'secondary', action: () => done('Dismissed without an answer.', { id: 'dismiss' }) }
        : { key: 'nothing', label: 'Nothing to do', words: 'none', tone: 'secondary', action: () => done(ANSWERS.nothing, typed(request, undefined, ANSWERS.nothing)) })
      return choices
    }
    return [
      ...(yesNo(alert.title)
        ? [
            { key: 'yes', label: 'Yes', words: 'none', tone: 'primary', action: () => done(ANSWERS.yes) },
            { key: 'no', label: 'No', words: 'none', tone: 'secondary', action: () => done(ANSWERS.no) },
          ] satisfies AlertChoice[]
        : []),
      { key: 'words', label: 'Send', words: 'required', tone: yesNo(alert.title) ? 'secondary' : 'primary', action: (words) => done(words.trim()) },
      { key: 'nothing', label: 'Nothing to do', words: 'none', tone: 'secondary', action: () => done(ANSWERS.nothing) },
    ]
  }
  if (alert.attention_kind === 'human-gate') {
    const choices: AlertChoice[] = []
    // Notes written on a review go with an approval too.
    if (offered.includes('review.approve'))
      choices.push({ key: 'approve', label: 'Approve', words: 'optional', tone: 'primary', action: (words) => ({ type: 'review.approve', reason: words.trim() }) })
    // A feedback gate offers request-changes where an approval gate offers reject.
    if (offered.includes('review.request-changes'))
      choices.push({ key: 'changes', label: 'Send back', words: 'required', tone: 'secondary', action: (words) => ({ type: 'review.request-changes', reason: words.trim() }) })
    if (offered.includes('review.reject'))
      choices.push({ key: 'reject', label: 'Reject', words: 'required', tone: 'danger', action: (words) => ({ type: 'review.reject', reason: words.trim() }) })
    return choices.length > 0 ? choices : null
  }
  if (alert.attention_kind === 'launch-approval') {
    const choices: AlertChoice[] = []
    if (offered.includes('launch.approve')) {
      // Approving launches the preview shown; a launch without one says why and cannot be approved.
      if (Option.isSome(alert.preview))
        choices.push({ key: 'approve', label: 'Approve launch', words: 'none', tone: 'primary', action: () => ({ type: 'launch.approve' }) })
      choices.push({ key: 'revise', label: 'Send back', words: 'required', tone: 'secondary', action: (words) => ({ type: 'launch.revise', feedback: words.trim() }) })
    }
    if (offered.includes('launch.cancel'))
      choices.push({ key: 'cancel', label: 'Cancel launch', words: 'none', tone: 'danger', action: () => ({ type: 'launch.cancel' }) })
    return choices.length > 0 ? choices : null
  }
  return null
}

/**
 * A request's link, only when it is an absolute http(s) URL. The URL is the asking agent's, so
 * any other scheme (javascript:, data:, …) or a relative path shows as plain text, never a link.
 */
export const webLink = (url: string | undefined): string | undefined => {
  if (url === undefined) return undefined
  try {
    const parsed = new URL(url)
    return parsed.protocol === 'http:' || parsed.protocol === 'https:' ? parsed.href : undefined
  } catch {
    return undefined
  }
}

/** Who an alert is from: the agent that asked, else the seat it is about. */
export const alertFrom = (alert: Pick<Native.Attention, 'requester_id' | 'source_id'>): string =>
  (alert.requester_id ?? alert.source_id).replace(/^agent\//, '')

export const KIND_WORDS: Record<string, string> = {
  'human-gate': 'Review',
  'person-step': 'Request',
  'agent-request': 'Request',
  'launch-approval': 'Launch approval',
  'revision-approval': 'Revision approval',
  'harness-login': 'Sign-in',
  'model-drift': 'Model drift',
}

/** The answers a card shows: options as one group, and the two that are not options. The recommended answer leads. */
export const partitionChoices = (choices: readonly AlertChoice[]): {
  readonly options: readonly AlertChoice[]
  /** The person's own words as an answer. */
  readonly words?: AlertChoice
  /** Setting it aside without an answer. */
  readonly dismiss?: AlertChoice
} => {
  const options = choices.filter((choice) => choice.key !== 'words' && choice.key !== 'dismiss' && choice.key !== 'nothing')
  const words = choices.find((choice) => choice.key === 'words')
  const dismiss = choices.find((choice) => choice.key === 'dismiss' || choice.key === 'nothing')
  return {
    options: [...options.filter((choice) => choice.recommended === true), ...options.filter((choice) => choice.recommended !== true)],
    ...(words === undefined ? {} : { words }),
    ...(dismiss === undefined ? {} : { dismiss }),
  }
}

/**
 * The answer Send would send. Words are never dropped: on an answer that cannot carry them they become the answer that
 * can, the person's own words where the ask takes them, else what asks for changes (as `personAnswer` routes words alone).
 */
export const effectiveChoice = (choices: readonly AlertChoice[], selected: string | undefined, words: string): AlertChoice | undefined => {
  const chosen = choices.find((choice) => choice.key === selected)
  if (words.trim() === '' || (chosen !== undefined && chosen.words !== 'none')) return chosen
  return choices.find((choice) => choice.key === 'words') ?? choices.find((choice) => choice.words === 'required') ?? chosen
}

/** Whether Send can send this answer with these words. Words nothing here can carry hold Send: they are never dropped. */
export const canSend = (choice: AlertChoice | undefined, words: string): boolean =>
  choice !== undefined && (words.trim() === '' ? choice.words !== 'required' : choice.words !== 'none')

/** How an answer reads in the record of it: its own label, or the person's words. */
export const answeredLabel = (choice: AlertChoice, words: string): string => {
  if (choice.key !== 'words') return choice.label
  const text = words.trim().replace(/\s+/g, ' ')
  return `“${text.length > 60 ? `${text.slice(0, 60)}…` : text}”`
}

/** The native prompt alerts of one agent's conversation, open ones only. */
export const promptAlerts = (items: readonly Native.Attention[], agentRef: string): readonly Native.Attention[] =>
  items.filter(
    (item) =>
      item.attention_kind === 'harness-prompt' &&
      item.state === 'open' &&
      alertConversations(item).includes(agentRef),
  )

/**
 * Every alert this agent's conversation shows, as the Fractal terminal UI and the phone choose them (`alertsIn`):
 * open, an alert, of a kind Home shows, and naming this conversation.
 */
export const conversationAlerts = (items: readonly Native.Attention[], agentRef: string): readonly Native.Attention[] =>
  alertsIn(items, agentRef, undefined)
