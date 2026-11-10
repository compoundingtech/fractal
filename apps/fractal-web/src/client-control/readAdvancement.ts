/**
 * Shared boundary for browser-local positions and daemon synchronization. The caller is the
 * pairing-authority adapter, never a URL parameter, browser preference or environment flag.
 * An enrollment ending does not turn the isolated test browser into an ordinary reader.
 */
export interface ReadAdvancementPolicy {
  readonly mayAdvance: () => boolean
  readonly prohibitForTestEnrollment: () => void
}

export const createReadAdvancementPolicy = (): ReadAdvancementPolicy => {
  let prohibited = false
  return {
    mayAdvance: () => !prohibited,
    prohibitForTestEnrollment: () => { prohibited = true },
  }
}

/**
 * Wrap the store's advance implementation, not its UI callers. Keep persistence and any outbound
 * synchronization inside `advance`: suppressing only onSeen would leave explicit mark-read writable.
 * The policy is checked at execution time, so already-created callbacks cannot bypass enrollment.
 */
export const gateReadAdvancement = <TArgs extends readonly unknown[], TResult>(
  policy: Pick<ReadAdvancementPolicy, 'mayAdvance'>,
  advance: (...args: TArgs) => TResult,
): ((...args: TArgs) => TResult | undefined) => (...args) =>
  policy.mayAdvance() ? advance(...args) : undefined
