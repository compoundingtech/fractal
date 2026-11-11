import type * as Native from '@fractal/client/schema'
import * as React from 'react'

import { promptAnswers } from '../../../../../clients/typescript/fractal-views/homeView.ts'
import { useAttention, useDataSource, useGrants } from '../../data/react.tsx'
import type { Feed } from '../../data/source.ts'
import type { UxTelemetry } from '../../telemetry/ux.ts'
import type { AlertAction, AlertActionResult } from './alert-action-port.ts'
import { AlertAnswersContext, hidesAlert, useAlertAnswersStore } from './alertAnswers.ts'
import type { PromptAnswerResult } from './prompt-answer-port.ts'

export type Act = (alert: Native.Attention, action: AlertAction) => Promise<AlertActionResult>
export type Answer = (alert: Native.Attention, answer: string, sent: string) => Promise<PromptAnswerResult>

/**
 * The one way an alert is answered: the active source's feed with in-flight answers hidden, and its answer ports with
 * `alert_landed` timing. The ports are absent when the source serves none or this device holds no action grant.
 */
export const useAlertAnswers = (ux?: UxTelemetry): {
  /** The feed as the source sent it. */
  readonly feed: Feed<readonly Native.Attention[]>
  readonly onAct?: Act
  readonly onAnswer?: Answer
} => {
  const source = useDataSource()
  const grants = useGrants()
  const feed = useAttention()
  const store = useAlertAnswersStore()
  // Every mount reports the same frames: the store settles each answer once.
  React.useLayoutEffect(() => { if (feed._tag === 'Observed') store.observe(feed.value) }, [feed, store])
  const answerable = grants.actions === 'granted'
  const actions = source.alertActions
  const onAct: Act | undefined = actions === undefined || !answerable ? undefined : (alert, action) => {
    const operation = ux?.beginAlertAnswer()
    return actions.act({ alert, action }).then((result) => {
      if (operation !== undefined) {
        if (result._tag === 'Sent') store.watchLanding(alert.id, operation)
        else operation.cancel()
      }
      return result
    })
  }
  const prompts = source.prompts
  const onAnswer: Answer | undefined = prompts === undefined || !answerable ? undefined : (alert, _answer, sent) => {
    const prompt = promptAnswers(alert)
    return prompt === null
      ? Promise.resolve({ _tag: 'Refused', detail: 'This prompt is answered in the seat\'s terminal.' })
      : prompts.respond({ alert: { id: alert.id, revision: alert.revision, source_id: alert.source_id, target_id: prompt.target_id, episode: prompt.episode }, answer: sent })
  }
  return { feed, ...(onAct === undefined ? {} : { onAct }), ...(onAnswer === undefined ? {} : { onAnswer }) }
}

/** The attention feed without alerts whose answer is on its way, for anything that counts or lists them. */
export const useVisibleAttention = (): Feed<readonly Native.Attention[]> => {
  const feed = useAttention()
  // Only what hides alerts: this feed's readers do not re-render on a draft's keystrokes.
  const store = React.useContext(AlertAnswersContext)
  const pending = React.useSyncExternalStore(store.subscribe, () => store.getSnapshot().pending)
  const settled = React.useSyncExternalStore(store.subscribe, () => store.getSnapshot().settled)
  return React.useMemo(() => feed._tag === 'Observed'
    ? { ...feed, value: feed.value.filter((item) => !hidesAlert(store.getSnapshot(), item)) }
    : feed, [feed, store, pending, settled])
}
