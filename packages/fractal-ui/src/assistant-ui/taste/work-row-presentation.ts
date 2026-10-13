import type { IconName } from '../composition/Icons.tsx'
import type { WorkKind, WorkLogCall } from './work-log.ts'

/** Presentation only: the host remains responsible for classifying tools. */
export const workKindPresentation: Readonly<Record<WorkKind, { readonly verb: string; readonly icon: IconName }>> = {
  read: { verb: 'Read', icon: 'folder' },
  search: { verb: 'Search', icon: 'search' },
  run: { verb: 'Run command', icon: 'drawer' },
  change: { verb: 'Change files', icon: 'pencil' },
  subagent: { verb: 'Delegate work', icon: 'swap' },
  web: { verb: 'Browse web', icon: 'search' },
  decision: { verb: 'Ask for a decision', icon: 'message' },
  todo: { verb: 'Update plan', icon: 'check' },
  wait: { verb: 'Wait for results', icon: 'clock' },
  message: { verb: 'Send message', icon: 'send' },
  schedule: { verb: 'Schedule work', icon: 'clock' },
  'terminal-input': { verb: 'Send terminal input', icon: 'drawer' },
  skill: { verb: 'Use skill', icon: 'folder' },
  eval: { verb: 'Evaluate code', icon: 'drawer' },
  unknown: { verb: 'System event', icon: 'gear' },
}

export interface WorkServicePresentation {
  readonly name: string
  readonly state?: string
  readonly readiness: 'Ready' | 'Timed out' | 'Not ready' | 'Readiness unreported'
  readonly warning: boolean
}
const record = (value: unknown): Readonly<Record<string, unknown>> | undefined =>
  typeof value === 'object' && value !== null && !Array.isArray(value) ? value as Readonly<Record<string, unknown>> : undefined

/** Only recorded service facts can replace a tool return's generic OK status. */
export const workServicePresentation = (call: WorkLogCall): WorkServicePresentation | undefined => {
  const output = record(call.output)
  const details = record(output?.details)
  const service = [record(call.metadata?.service), record(call.view?.service), call.view?.type === 'service' ? call.view : undefined, record(details?.service)]
    .find(value => typeof value?.name === 'string')
  if (service === undefined || typeof service.name !== 'string') return undefined
  return {
    name: service.name,
    ...(typeof service.state === 'string' ? { state: service.state } : {}),
    readiness: service.timedOut === true ? 'Timed out' : service.ready === true ? 'Ready' : service.ready === false ? 'Not ready' : 'Readiness unreported',
    warning: service.timedOut === true || service.ready === false,
  }
}
