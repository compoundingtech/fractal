import { recordValue } from './blocks.ts'

export interface RecordedWait {
  readonly awaited: readonly string[]
  readonly arrived: readonly string[]
  readonly stillRunning: readonly string[]
  readonly senders: readonly string[]
  readonly waited?: number
}
const names = (value: unknown): readonly string[] => typeof value === 'string' ? [value] : Array.isArray(value) ? value.flatMap(entry => {
  if (typeof entry === 'string') return [entry]
  const record = recordValue(entry), name = record?.ref ?? record?.jobId ?? record?.id ?? record?.name
  return typeof name === 'string' ? [name] : []
}) : []

/** A jobs snapshot is not a delivery list; metadata.from identifies the waiting seat. */
export const recordedWaitFor = ({ input: rawInput, view, metadata, arrived, awaited }: {
  readonly input?: unknown
  readonly view?: Readonly<Record<string, unknown>>
  readonly metadata?: Readonly<Record<string, unknown>>
  readonly arrived?: unknown
  readonly awaited?: unknown
}): RecordedWait => {
  const input = recordValue(rawInput), waited = metadata?.waited, message = recordValue(waited)
  const senders = typeof message?.from === 'string' ? [message.from] : []
  const snapshot = view?.jobs ?? metadata?.jobs
  const jobs = Array.isArray(snapshot) ? snapshot : []
  const completed = jobs.filter(job => recordValue(job)?.status === 'completed')
  const stillRunning = names(jobs.filter(job => recordValue(job)?.status === 'running'))
  // Native arrived is an explicit delivery fact, unlike the legacy jobs snapshot.
  const deliveries = view?.arrived ?? (Array.isArray(snapshot) ? completed : metadata?.arrived ?? arrived)
  return {
    awaited: names(awaited ?? view?.awaited ?? input?.awaited ?? input?.jobs ?? input?.handles ?? input?.ids),
    arrived: [...names(deliveries), ...senders],
    stillRunning,
    senders,
    ...(typeof waited === 'number' && Number.isFinite(waited) ? { waited } : {}),
  }
}
