/** WebSocket events are tasks, not microtasks. MessageChannel is not a timer, so
 * deliveries still run while Playwright's clock is paused. */
export const deliveryTasks = () => {
  const channel = new MessageChannel()
  let pending: { run: () => void; preserveOnClear: boolean }[] = []
  const waiters = new Set<() => void>()
  let running = false
  let closed = false
  channel.port1.onmessage = () => {
    const task = pending.shift()
    running = true
    try { task?.run() } finally {
      running = false
      if (pending.length === 0) {
        if ('unref' in channel.port1 && typeof channel.port1.unref === 'function') channel.port1.unref()
        for (const resolve of waiters) resolve()
        waiters.clear()
      }
    }
  }
  // Idle ports must not keep Node/Bun alive; pending work refs the receiving port.
  for (const port of [channel.port1, channel.port2]) {
    if ('unref' in port && typeof port.unref === 'function') port.unref()
  }
  return {
    enqueue: (run: () => void, { preserveOnClear = false }: { preserveOnClear?: boolean } = {}): void => {
      if (closed) return
      pending.push({ run, preserveOnClear })
      if ('ref' in channel.port1 && typeof channel.port1.ref === 'function') channel.port1.ref()
      channel.port2.postMessage(null)
    },
    flush: (): Promise<void> => {
      if (pending.length === 0 && !running) return Promise.resolve()
      // This package targets ES2022, before Promise.withResolvers.
      return new Promise<void>((resolve) => { waiters.add(resolve) })
    },
    clear: (): void => { pending = pending.filter((task) => task.preserveOnClear) },
    close: (): void => {
      closed = true
      channel.port1.close()
      channel.port2.close()
    },
  }
}
