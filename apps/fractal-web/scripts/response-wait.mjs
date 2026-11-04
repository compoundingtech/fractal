// A page response collected from the moment this is called, with no timer until something waits on it.
// page.waitForResponse starts its timeout when it is created, so one created before a long setup and
// awaited after it can reject first, and that unhandled rejection ends the whole browser run (#4244).
export function collectResponse(page, matches) {
  let onResponse
  const response = new Promise(resolve => {
    onResponse = candidate => {
      if (!matches(candidate)) return
      page.off('response', onResponse)
      resolve(candidate)
    }
    page.on('response', onResponse)
  })
  return {
    // The collected response, or a rejection `ms` after this call when it has not arrived.
    within(ms, what) {
      let deadline
      const late = new Promise((_, reject) => {
        deadline = setTimeout(() => {
          page.off('response', onResponse)
          reject(new Error(`${what} did not arrive within ${ms} ms`))
        }, ms)
      })
      return Promise.race([response, late]).finally(() => clearTimeout(deadline))
    },
  }
}
