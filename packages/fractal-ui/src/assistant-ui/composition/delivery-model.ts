export type JsonValue = string | number | boolean | null | readonly JsonValue[] | { readonly [key: string]: JsonValue }
export type DeliveryAction = { readonly _tag: 'Available'; readonly onPress: () => void } | { readonly _tag: 'Unavailable'; readonly reason: string }
export type OutputContent = { readonly _tag: 'Complete'; readonly text: string } | { readonly _tag: 'Preview'; readonly text: string; readonly earlierLines: number; readonly fullOutput: DeliveryAction }
export type AgentResultState = { readonly _tag: 'Completed' } | { readonly _tag: 'Running' } | { readonly _tag: 'Cancelled'; readonly reason: string } | { readonly _tag: 'Aborted'; readonly reason: string } | { readonly _tag: 'Failed'; readonly error: string; readonly exitCode?: number }
export type AgentLifecycle = { readonly _tag: 'Idle' } | { readonly _tag: 'Stopped'; readonly resumable: boolean } | { readonly _tag: 'Aborted' } | { readonly _tag: 'Isolated' } | { readonly _tag: 'Submitted' }
export type ResultFact = { readonly _tag: 'Commit'; readonly value: string } | { readonly _tag: 'Branch'; readonly value: string } | { readonly _tag: 'Pushed'; readonly value: boolean } | { readonly _tag: 'Checks'; readonly value: string } | { readonly _tag: 'ExitCode'; readonly value: number } | { readonly _tag: 'Count'; readonly label: string; readonly value: number }
export interface AgentResultData {
  readonly id: string
  readonly name: string
  readonly agentKind?: string
  readonly state: AgentResultState
  readonly schema: { readonly _tag: 'Valid' } | { readonly _tag: 'Invalid'; readonly reason: string } | { readonly _tag: 'NotProvided' }
  readonly excerpt?: string
  readonly facts: readonly ResultFact[]
  readonly report: OutputContent
  readonly raw: JsonValue
  readonly lifecycle: AgentLifecycle
  readonly durationMs?: number
  readonly time?: string
  readonly openThread: DeliveryAction
  readonly messageReason: string
  readonly resumeReason: string
}
export type BackgroundJobState = { readonly _tag: 'Exited'; readonly exit: { readonly _tag: 'Reported'; readonly code: number } | { readonly _tag: 'NotReported' } } | { readonly _tag: 'Ready' } | { readonly _tag: 'Monitor'; readonly event: string } | { readonly _tag: 'Cancelled'; readonly reason: string } | { readonly _tag: 'Aborted'; readonly reason: string }
export interface BackgroundJobData {
  readonly id: string
  readonly command: string
  readonly kind: 'command' | 'service'
  readonly state: BackgroundJobState
  readonly durationMs?: number
  readonly output: OutputContent
  readonly startedBy?: { readonly label: string; readonly onPress: () => void }
}
export type DeliveryChild = { readonly _tag: 'Agent'; readonly result: AgentResultData } | { readonly _tag: 'Job'; readonly job: BackgroundJobData }
export interface HarnessDeliveryData { readonly id: string; readonly time?: string; readonly children: readonly DeliveryChild[] }
