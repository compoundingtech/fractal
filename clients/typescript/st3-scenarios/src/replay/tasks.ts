/** WebSocket events are tasks, not microtasks. MessageChannel is not a timer, so
 * deliveries still run while Playwright's clock is paused. */
export const deliveryTasks = () => {
  const channel = new MessageChannel()
  const pending: (() => void)[] = []
  const waiters = new Set<() => void>()
  let running = false
  let closed = false
  channel.port1.onmessage = () => {
    const task = pending.shift()
    running = true
    try { task?.() } finally {
      running = false
      if (pending.length === 0) {
        for (const resolve of waiters) resolve()
        waiters.clear()
      }
    }
  }
  // Node and Bun expose unref; browser MessagePorts do not.
  for (const port of [channel.port1, channel.port2]) {
    if ('unref' in port && typeof port.unref === 'function') port.unref()
  }
  return {
    enqueue: (task: () => void): void => {
      if (closed) return
      pending.push(task)
      channel.port2.postMessage(null)
    },
    flush: (): Promise<void> => {
      if (pending.length === 0 && !running) return Promise.resolve()
      // This package targets ES2022, before Promise.withResolvers.
      return new Promise<void>((resolve) => { waiters.add(resolve) })
    },
    clear: (): void => { pending.length = 0 },
    close: (): void => {
      closed = true
      channel.port1.close()
      channel.port2.close()
    },
  }
}
